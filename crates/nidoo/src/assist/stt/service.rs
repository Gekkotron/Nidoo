//! The voice service: hears the satellites over UDP, runs one [`Session`] per satellite, and
//! carries out what the sessions ask: hold the room's music through `nidoo-sound`, close or
//! reopen a board's window, transcribe, ask the assistant and speak its reply on the
//! satellite that heard the question. Audio only lives in memory, for one phrase.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nidoo_assist::view::AssistRequest;
use nidoo_core::command::Publish;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

use super::Stt;
use super::ask::{AskOutcome, AskRequest};
use super::engine::Vad;
use super::guard::{HubCounter, ReplayGuard};
use super::packet::{BoardMsg, HubMsg, WakeKind, decode_board, encode_hub};
use super::session::{Input, Output, Rooms, Session, SessionCfg};
use crate::hub::Hub;
use crate::sound::{
    HOLD_TOPIC, SpeakTarget, room_of, room_satellites, speaking_satellites, voice_satellites,
};

/// The UDP port every board listens on.
pub const BOARD_PORT: u16 = 4961;
/// How often the hub greets the boards.
pub const HELLO_EVERY: Duration = Duration::from_secs(5);
/// How long one hold lasts unless refreshed.
pub const HOLD_MS: u32 = 30_000;
/// The sessions' clock tick.
const TICK: Duration = Duration::from_millis(50);

/// Listens on the configured port and serves the boards on [`BOARD_PORT`].
pub async fn run_voice(hub: Arc<Hub>, stt: Arc<Stt>) -> anyhow::Result<()> {
    let port = stt.port_in_use();
    let socket = UdpSocket::bind(("0.0.0.0", port)).await?;
    tracing::info!(port, "speech input listens for the satellites");
    run_voice_on(hub, stt, socket, BOARD_PORT).await
}

/// Asks the assistant for `sid`'s phrase and speaks the reply on that satellite: the clip
/// being spoken (none when nothing is said) and whether the reply is a question.
pub type AskFn = Arc<
    dyn Fn(
            Arc<Hub>,
            String,
            String,
        ) -> futures_util::future::BoxFuture<'static, (Option<String>, bool)>
        + Send
        + Sync,
>;

/// A real turn (`satellite:<sid>`, not simulated), its reply said on that satellite.
pub fn default_ask() -> AskFn {
    Arc::new(|hub: Arc<Hub>, sid: String, text: String| {
        Box::pin(async move {
            let request = AssistRequest {
                client: format!("satellite:{sid}"),
                text: Some(text),
                reply: None,
                simulate: false,
                lang: None,
            };
            match hub.assist(request).await {
                Ok(turn) => {
                    let target = SpeakTarget::Satellite(sid.clone());
                    let clip = match hub.speak_clip(target, &turn.reply, None, None).await {
                        Ok((clip, _)) => Some(clip),
                        Err(e) => {
                            tracing::info!(%sid, "the reply is not spoken: {e}");
                            None
                        }
                    };
                    (clip, turn.question.is_some())
                }
                Err(e) => {
                    tracing::warn!(%sid, "voice turn refused: {e}");
                    (None, false)
                }
            }
        })
    })
}

/// [`run_voice`] on a bound socket, with the boards on `board_port` (tests).
pub async fn run_voice_on(
    hub: Arc<Hub>,
    stt: Arc<Stt>,
    socket: UdpSocket,
    board_port: u16,
) -> anyhow::Result<()> {
    run_voice_with(hub, stt, socket, board_port, default_ask()).await
}

/// [`run_voice_on`] with another way to ask and answer (tests).
pub async fn run_voice_with(
    hub: Arc<Hub>,
    stt: Arc<Stt>,
    socket: UdpSocket,
    board_port: u16,
    ask: AskFn,
) -> anyhow::Result<()> {
    let socket = Arc::new(socket);
    let (events, mut results) = mpsc::channel(64);
    let mut spoken = hub.subscribe_spoken();
    let (ask_tx, mut asks) = mpsc::channel::<AskRequest>(8);
    if hub.voice_asks.set(ask_tx).is_err() {
        tracing::debug!("the hub already has a voice service for its questions");
    }
    let mut voice = Voice {
        hub,
        stt,
        socket: Arc::clone(&socket),
        board_port,
        sats: HashMap::new(),
        rooms: Rooms::default(),
        counter: HubCounter::default(),
        started: Instant::now(),
        events,
        refused: 0,
        ask,
        pending: HashMap::new(),
        next_ask: 0,
    };
    let mut hello = tokio::time::interval(HELLO_EVERY);
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut buf = vec![0u8; 2048];
    loop {
        tokio::select! {
            got = socket.recv_from(&mut buf) => match got {
                Ok((n, from)) => voice.on_datagram(&buf[..n], from).await,
                Err(e) => tracing::debug!("voice socket: {e}"),
            },
            _ = hello.tick() => voice.greet().await,
            _ = tick.tick() => voice.on_tick().await,
            Some(event) = results.recv() => voice.on_event(event).await,
            Some(request) = asks.recv() => voice.on_ask(request).await,
            report = spoken.recv() => match report {
                Ok(report) => {
                    let clip = report.id;
                    voice.feed_all(|| Input::Spoken { clip: clip.clone() }).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
            },
        }
    }
}

/// What becomes of a wake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Not a satellite the hub greets, or a stale or replayed wake: no answer at all.
    Unknown,
    /// Signed and fresh, but the hub does not take it: the board is told to close.
    Refused,
    /// Fed to the satellite's session.
    Admitted,
}

/// What a background job found.
enum Event {
    Transcript {
        sid: String,
        text: String,
    },
    Replied {
        sid: String,
        clip: Option<String>,
        question: bool,
    },
    /// The clip of the hub's own question (none when it could not be made).
    AskClip {
        sid: String,
        /// The question it belongs to: a clip of a cancelled question is dropped.
        id: u64,
        clip: Option<String>,
    },
}

/// A question of the hub being asked on some satellites.
struct Pending {
    reply: oneshot::Sender<AskOutcome>,
    /// The satellites that have not given up yet.
    sats: Vec<String>,
}

/// One satellite the hub greets.
struct Sat {
    session: Session,
    vad: Box<dyn Vad>,
    guard: ReplayGuard,
    /// Where its board listens.
    addr: SocketAddrV4,
    /// The exchange's stages, in ms since its wake word.
    stages: Vec<(&'static str, u64)>,
    /// The hub's question this satellite is asking, if any.
    ask_id: Option<u64>,
}

struct Voice {
    hub: Arc<Hub>,
    stt: Arc<Stt>,
    socket: Arc<UdpSocket>,
    board_port: u16,
    sats: HashMap<String, Sat>,
    rooms: Rooms,
    counter: HubCounter,
    started: Instant,
    events: mpsc::Sender<Event>,
    /// Datagrams refused so far (logged now and then).
    refused: u64,
    ask: AskFn,
    /// Questions of the hub in flight, by id.
    pending: HashMap<u64, Pending>,
    next_ask: u64,
}

fn unix_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
}

/// The hub's own address on the way to `board` (the system's route, nothing is sent).
fn local_ip_towards(board: Ipv4Addr) -> Option<Ipv4Addr> {
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    probe.connect((board, 9)).ok()?;
    match probe.local_addr().ok()? {
        SocketAddr::V4(addr) => Some(*addr.ip()),
        SocketAddr::V6(_) => None,
    }
}

fn session_cfg(stt: &Stt) -> SessionCfg {
    let settings = stt.snapshot().settings;
    SessionCfg {
        gain: settings.gain,
        listen_margin_ms: u64::from(settings.listen_margin_ms),
    }
}

impl Voice {
    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Greets every satellite that can take part, with a fresh nonce each.
    async fn greet(&mut self) {
        let snapshot = self.stt.snapshot();
        let (Some(key), Some(engines)) = (snapshot.key, self.stt.engines()) else {
            return;
        };
        let Ok(local) = self.socket.local_addr() else {
            return;
        };
        let targets = voice_satellites(&self.hub.read());
        for (sid, ip) in targets {
            let Some(hub_ip) = local_ip_towards(ip) else {
                continue;
            };
            let addr = SocketAddrV4::new(ip, self.board_port);
            let sat = self.sats.entry(sid.clone()).or_insert_with(|| Sat {
                session: Session::new(session_cfg(&self.stt)),
                vad: engines.vads.new_vad(),
                guard: ReplayGuard::default(),
                addr,
                stages: Vec::new(),
                ask_id: None,
            });
            sat.addr = addr;
            let mut nonce = [0u8; 8];
            if getrandom::getrandom(&mut nonce).is_err() {
                continue;
            }
            let nonce = u64::from_le_bytes(nonce);
            sat.guard.note_hello(nonce);
            let msg = HubMsg::Hello {
                nonce,
                hub_ctr: self.counter.next(unix_us()),
                hub: SocketAddrV4::new(hub_ip, local.port()),
            };
            let bytes = encode_hub(&key, &sid, &msg);
            if let Err(e) = self.socket.send_to(&bytes, addr).await {
                tracing::debug!(%sid, "hello not sent: {e}");
            }
        }
    }

    async fn on_datagram(&mut self, datagram: &[u8], from: SocketAddr) {
        let Some(key) = self.stt.snapshot().key else {
            return;
        };
        let Ok((sid, msg)) = decode_board(&key, datagram) else {
            self.refused += 1;
            if self.refused.is_power_of_two() {
                tracing::debug!(%from, refused = self.refused, "voice datagram refused");
            }
            return;
        };
        let now = self.now();
        let input = match msg {
            BoardMsg::Wake {
                nonce,
                session,
                kind,
                listen_ctr,
            } => {
                match self.admit_wake(&sid, nonce, session, kind, now) {
                    Verdict::Unknown => return,
                    Verdict::Refused => {}
                    Verdict::Admitted => {
                        let wake = Input::Wake {
                            session,
                            kind,
                            listen_ctr,
                        };
                        self.feed(&sid, wake).await;
                    }
                }
                // A board whose window the hub does not take closes it at once: its speaker
                // and LED come back, and its next Listen is not lost in a useless window.
                let listening = self
                    .sats
                    .get(&sid)
                    .is_some_and(|sat| sat.session.is_listening(session));
                if !listening {
                    let hub_ctr = self.counter.next(unix_us());
                    self.send(&sid, &HubMsg::Stop { hub_ctr, session }).await;
                }
                return;
            }
            BoardMsg::Audio {
                session,
                seq,
                samples,
            } => Input::Audio {
                session,
                seq,
                samples,
            },
            BoardMsg::End { session } => Input::End { session },
        };
        self.feed(&sid, input).await;
    }

    /// A signed wake from a greeted satellite with a fresh nonce is `Refused` or
    /// `Admitted`; a wake word is admitted when the satellite may still take part and its
    /// room has no exchange yet, which starts a fresh exchange.
    fn admit_wake(
        &mut self,
        sid: &str,
        nonce: u64,
        session: u32,
        kind: WakeKind,
        now: u64,
    ) -> Verdict {
        let Some(sat) = self.sats.get_mut(sid) else {
            return Verdict::Unknown;
        };
        if !sat.guard.accept_wake(nonce, session) {
            return Verdict::Unknown;
        }
        if kind == WakeKind::FollowUp {
            return Verdict::Admitted;
        }
        let Some(engines) = self.stt.engines() else {
            return Verdict::Refused;
        };
        if !sat.session.is_idle() {
            return Verdict::Refused;
        }
        let (able, room) = {
            let reg = self.hub.read();
            let able = voice_satellites(&reg).iter().any(|(s, _)| s == sid);
            (able, room_of(&reg, sid).unwrap_or_else(|| sid.to_owned()))
        };
        if !able || !self.rooms.admit(sid, &room) {
            return Verdict::Refused;
        }
        sat.session = Session::new(session_cfg(&self.stt));
        sat.vad = engines.vads.new_vad();
        sat.stages = vec![("wake", now)];
        Verdict::Admitted
    }

    async fn on_tick(&mut self) {
        self.feed_all(|| Input::Tick).await;
        // A question nobody waits for any more (its flow run ended) is given up.
        let closed: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, p)| p.reply.is_closed())
            .map(|(id, _)| *id)
            .collect();
        for id in closed {
            self.cancel_ask(id, None).await;
        }
    }

    /// Drops question `id` and ends its satellites' exchanges, except `keep`.
    async fn cancel_ask(&mut self, id: u64, keep: Option<&str>) -> Option<Pending> {
        let pending = self.pending.remove(&id)?;
        for sid in &pending.sats {
            if Some(sid.as_str()) != keep {
                self.feed(sid, Input::AskCancel).await;
            }
        }
        Some(pending)
    }

    /// Asks the hub's question on the free satellites of the request's target.
    async fn on_ask(&mut self, request: AskRequest) {
        let AskRequest {
            target,
            text,
            reply,
        } = request;
        if reply.is_closed() {
            // The run was cancelled before the question was taken: say nothing.
            return;
        }
        let candidates = {
            let reg = self.hub.read();
            match speaking_satellites(&reg, &target) {
                Ok(speaking) => {
                    let able = voice_satellites(&reg);
                    Ok(speaking
                        .into_iter()
                        .filter(|sid| able.iter().any(|(s, _)| s == sid))
                        .map(|sid| {
                            let room = room_of(&reg, &sid).unwrap_or_else(|| sid.clone());
                            (sid, room)
                        })
                        .collect::<Vec<_>>())
                }
                Err(e) => Err(e.to_string()),
            }
        };
        let candidates = match candidates {
            Ok(c) => c,
            Err(why) => {
                let _ = reply.send(AskOutcome::NoSatellite(why));
                return;
            }
        };
        let Some(engines) = self.stt.engines() else {
            let _ = reply.send(AskOutcome::Off);
            return;
        };
        let mut sats = Vec::new();
        for (sid, room) in candidates {
            let Some(sat) = self.sats.get_mut(&sid) else {
                continue;
            };
            if !sat.session.is_idle() || !self.rooms.admit(&sid, &room) {
                continue;
            }
            sat.session = Session::new(session_cfg(&self.stt));
            sat.vad = engines.vads.new_vad();
            sats.push(sid);
        }
        if sats.is_empty() {
            let why = match &target {
                SpeakTarget::Room(room) => format!("no free satellite in {room}"),
                _ => "no free satellite".to_owned(),
            };
            let _ = reply.send(AskOutcome::NoSatellite(why));
            return;
        }
        let id = self.next_ask;
        self.next_ask += 1;
        self.pending.insert(
            id,
            Pending {
                reply,
                sats: sats.clone(),
            },
        );
        for sid in sats {
            if let Some(sat) = self.sats.get_mut(&sid) {
                sat.ask_id = Some(id);
            }
            self.mark(&sid, "ask");
            self.feed(&sid, Input::AskStart).await;
            let hub = Arc::clone(&self.hub);
            let events = self.events.clone();
            let text = text.clone();
            tokio::spawn(async move {
                let clip = match hub
                    .speak_clip(SpeakTarget::Satellite(sid.clone()), &text, None, None)
                    .await
                {
                    Ok((clip, _)) => Some(clip),
                    Err(e) => {
                        tracing::info!(%sid, "the question is not spoken: {e}");
                        None
                    }
                };
                let _ = events.send(Event::AskClip { sid, id, clip }).await;
            });
        }
    }

    /// The asking session of `sid` gave its answer.
    async fn on_answer(&mut self, sid: &str, answer: Option<String>) {
        let Some(id) = self.sats.get_mut(sid).and_then(|sat| sat.ask_id.take()) else {
            return;
        };
        match answer {
            Some(text) => {
                // `feed` -> `run` -> here -> `feed`: the loop needs a box.
                if let Some(pending) = Box::pin(self.cancel_ask(id, Some(sid))).await {
                    let _ = pending.reply.send(AskOutcome::Heard {
                        sid: sid.to_owned(),
                        text,
                    });
                }
            }
            None => {
                let Some(pending) = self.pending.get_mut(&id) else {
                    return;
                };
                pending.sats.retain(|s| s != sid);
                if pending.sats.is_empty()
                    && let Some(pending) = self.pending.remove(&id)
                {
                    let _ = pending.reply.send(AskOutcome::Silence);
                }
            }
        }
    }

    async fn on_event(&mut self, event: Event) {
        match event {
            Event::Transcript { sid, text } => {
                self.mark(&sid, "text");
                self.feed(&sid, Input::Transcript(text)).await;
            }
            Event::Replied {
                sid,
                clip,
                question,
            } => {
                self.mark(&sid, "reply");
                self.feed(&sid, Input::Replied { clip, question }).await;
            }
            Event::AskClip { sid, id, clip } => {
                if self.sats.get(&sid).and_then(|sat| sat.ask_id) == Some(id) {
                    self.feed(&sid, Input::AskClip(clip)).await;
                }
            }
        }
    }

    fn mark(&mut self, sid: &str, stage: &'static str) {
        let now = self.now();
        if let Some(sat) = self.sats.get_mut(sid) {
            sat.stages.push((stage, now));
        }
    }

    async fn feed_all(&mut self, input: impl Fn() -> Input) {
        let sids: Vec<String> = self.sats.keys().cloned().collect();
        for sid in sids {
            self.feed(&sid, input()).await;
        }
    }

    async fn feed(&mut self, sid: &str, input: Input) {
        let now = self.now();
        let Some(sat) = self.sats.get_mut(sid) else {
            return;
        };
        let outputs = sat.session.on(input, now, sat.vad.as_mut());
        for output in outputs {
            self.run(sid, output).await;
        }
    }

    async fn run(&mut self, sid: &str, output: Output) {
        match output {
            Output::Hold => self.hold(sid, HOLD_MS),
            Output::Answer(answer) => self.on_answer(sid, answer).await,
            Output::Release => {
                if let Some(sat) = self.sats.get_mut(sid) {
                    sat.ask_id = None;
                }
                self.hold(sid, 0);
                self.rooms.done(sid);
                self.log_exchange(sid);
            }
            Output::Stop { session } => {
                self.mark(sid, "phrase");
                let hub_ctr = self.counter.next(unix_us());
                self.send(sid, &HubMsg::Stop { hub_ctr, session }).await;
            }
            Output::Listen => {
                let hub_ctr = self.counter.next(unix_us());
                self.send(sid, &HubMsg::Listen { hub_ctr }).await;
                if let Some(sat) = self.sats.get_mut(sid) {
                    sat.session.listening(hub_ctr);
                }
            }
            Output::Transcribe(phrase) => {
                let engines = self.stt.engines();
                let events = self.events.clone();
                let sid = sid.to_owned();
                tokio::task::spawn_blocking(move || {
                    let text =
                        engines.map_or_else(String::new, |e| e.transcriber.transcribe(&phrase));
                    tracing::debug!(%sid, %text, "heard");
                    let _ = events.blocking_send(Event::Transcript { sid, text });
                });
            }
            Output::Ask(text) => {
                let ask = Arc::clone(&self.ask);
                let hub = Arc::clone(&self.hub);
                let events = self.events.clone();
                let sid = sid.to_owned();
                tokio::spawn(async move {
                    let (clip, question) = ask(hub, sid.clone(), text).await;
                    let _ = events
                        .send(Event::Replied {
                            sid,
                            clip,
                            question,
                        })
                        .await;
                });
            }
        }
    }

    /// Holds (or, with 0, releases) the music of `sid`'s room; the hold's id is `sid`.
    fn hold(&self, sid: &str, for_ms: u32) {
        let players = room_satellites(&self.hub.read(), sid);
        let payload = serde_json::json!({ "id": sid, "players": players, "for_ms": for_ms });
        let publish = Publish {
            topic: HOLD_TOPIC.to_owned(),
            payload: payload.to_string(),
            retain: false,
        };
        if let Err(e) = self.hub.publish(publish) {
            tracing::warn!(%sid, "hold not sent: {e}");
        }
    }

    async fn send(&self, sid: &str, msg: &HubMsg) {
        let (Some(key), Some(sat)) = (self.stt.snapshot().key, self.sats.get(sid)) else {
            return;
        };
        if let Err(e) = self
            .socket
            .send_to(&encode_hub(&key, sid, msg), sat.addr)
            .await
        {
            tracing::debug!(%sid, "voice message not sent: {e}");
        }
    }

    /// One line per exchange: how long each stage took (no words said).
    fn log_exchange(&mut self, sid: &str) {
        let now = self.now();
        let Some(sat) = self.sats.get_mut(sid) else {
            return;
        };
        let mut stages = std::mem::take(&mut sat.stages);
        let Some(&(_, start)) = stages.first() else {
            return;
        };
        stages.push(("end", now));
        let steps: Vec<String> = stages
            .windows(2)
            .map(|w| format!("{} +{} ms", w[1].0, w[1].1.saturating_sub(w[0].1)))
            .collect();
        tracing::info!(%sid, total_ms = now.saturating_sub(start), "voice exchange: {}", steps.join(", "));
    }
}

#[cfg(test)]
mod tests;
