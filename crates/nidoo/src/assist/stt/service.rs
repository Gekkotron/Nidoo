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
use tokio::sync::mpsc;

use super::Stt;
use super::engine::Vad;
use super::guard::{HubCounter, ReplayGuard};
use super::packet::{BoardMsg, HubMsg, WakeKind, decode_board, encode_hub};
use super::session::{Input, Output, Rooms, Session, SessionCfg};
use crate::hub::Hub;
use crate::sound::{HOLD_TOPIC, SpeakTarget, room_of, room_satellites, voice_satellites};

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
            Output::Answer(_) => {}
            Output::Release => {
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
mod tests {
    use super::super::engine::{EngineLoader, Engines, fakes};
    use super::super::models::SttModels;
    use super::super::packet::{Key, decode_hub, encode_board};
    use super::super::store;
    use super::*;
    use crate::config::Config;
    use nidoo_api::ConnectionStatus;
    use nidoo_core::model::{Area, AreasDocument, SOUND_DEVICE_PREFIX};
    use serde_json::json;
    use std::path::Path;
    use std::sync::Mutex;

    const SID: &str = "0123456789ab";
    const LED_FIXTURE: &str = "light__0x60a423fffe94d1c6__light.json";
    const LED_TOPIC: &str = "nidoo/light/0x60a423fffe94d1c6/light/config";
    const LED_COMMAND_TOPIC: &str = "zigbee2mqtt/rooms/living/led_ceiling/set";
    const WAIT: Duration = Duration::from_secs(8);

    struct Heard(&'static str);

    impl EngineLoader for Heard {
        fn built(&self) -> bool {
            true
        }

        fn load(&self, _: &Path, _: &Path, _: u8) -> Result<Engines, String> {
            Ok(fakes::engines(self.0))
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../nidoo-core/tests/fixtures/discovery")
                .join(name),
        )
        .unwrap()
    }

    async fn place(hub: &Hub, id: &str, area: &str) {
        let mut patch = serde_json::Map::new();
        patch.insert("area".to_owned(), json!(area));
        hub.set_override(id, &patch).await.expect("area");
    }

    /// A house with the living room's light and one Allowed satellite at 127.0.0.1 in it;
    /// every publish is kept.
    async fn house() -> (Arc<Hub>, Arc<Mutex<Vec<Publish>>>) {
        let mut cfg = Config::default();
        cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
        cfg.commands.confirm_timeout_ms = 50;
        let (tx, mut rx) = mpsc::channel(256);
        let hub = Hub::new(&cfg, tx);
        hub.set_connection(ConnectionStatus::Online);
        hub.set_areas(AreasDocument {
            areas: vec![Area {
                id: "living".into(),
                name: "Salon".into(),
                ..Area::default()
            }],
        })
        .await
        .unwrap();
        hub.ingest(LED_TOPIC, &fixture(LED_FIXTURE));
        let led = hub
            .snapshot()
            .entities
            .into_iter()
            .find(|e| e.id.starts_with("0x60a423fffe94d1c6"))
            .unwrap()
            .id;
        place(&hub, &led, "living").await;
        hub.ingest(
            "zigbee2mqtt/rooms/living/led_ceiling",
            br#"{"state":"OFF","brightness":100}"#,
        );
        add_satellite(&hub, SID).await;
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        tokio::spawn(async move {
            while let Some(p) = rx.recv().await {
                sink.lock().unwrap().push(p);
            }
        });
        (hub, log)
    }

    /// An Allowed, connected satellite at 127.0.0.1 in the living room.
    async fn add_satellite(hub: &Hub, sid: &str) {
        let device = format!("{SOUND_DEVICE_PREFIX}{sid}");
        for (component, object) in [
            ("number", "volume"),
            ("switch", "mute"),
            ("switch", "allowed"),
            ("binary_sensor", "connected"),
            ("sensor", "address"),
        ] {
            let mut config = json!({
                "name": object,
                "unique_id": format!("{device}_{object}"),
                "state_topic": format!("nidoo-sound/{sid}/{object}"),
                "availability_topic": "nidoo-sound/status",
                "device": { "identifiers": [device], "name": sid },
            });
            if matches!(component, "switch" | "binary_sensor") {
                config["payload_on"] = json!("ON");
                config["payload_off"] = json!("OFF");
            }
            if component != "binary_sensor" && component != "sensor" {
                config["command_topic"] = json!(format!("nidoo-sound/{sid}/{object}/set"));
            }
            hub.ingest(
                &format!("nidoo/{component}/{device}/{object}/config"),
                config.to_string().as_bytes(),
            );
        }
        place(hub, &format!("{device}_volume"), "living").await;
        hub.ingest("nidoo-sound/status", b"online");
        for (object, value) in [
            ("volume", "40"),
            ("mute", "OFF"),
            ("allowed", "ON"),
            ("connected", "ON"),
            ("address", "127.0.0.1"),
        ] {
            hub.ingest(&format!("nidoo-sound/{sid}/{object}"), value.as_bytes());
        }
    }

    /// Speech input with the model "installed" and engines that always hear `heard`.
    async fn stt(dir: &Path, with_key: bool, heard: &'static str) -> Arc<Stt> {
        let models = dir.join(store::MODEL_DIR);
        std::fs::create_dir_all(&models).unwrap();
        for f in store::MODEL_FILES {
            std::fs::write(models.join(f), b"x").unwrap();
        }
        std::fs::write(dir.join(store::VAD_FILE), b"x").unwrap();
        let stt = Stt::start_with(
            dir,
            Vec::new(),
            SttModels::default(),
            Arc::new(Heard(heard)),
        );
        if with_key {
            stt.new_key().unwrap();
        }
        for _ in 0..100 {
            if stt.engines().is_some() {
                return stt;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("engines never loaded")
    }

    struct Board {
        socket: UdpSocket,
        key: Key,
    }

    impl Board {
        async fn recv(&self) -> Option<HubMsg> {
            let mut buf = [0u8; 2048];
            let (n, _) = tokio::time::timeout(WAIT, self.socket.recv_from(&mut buf))
                .await
                .ok()?
                .ok()?;
            decode_hub(&self.key, &buf[..n]).ok().map(|(_, m)| m)
        }

        async fn send(&self, to: SocketAddrV4, msg: BoardMsg, key: &Key) {
            self.send_as(SID, to, msg, key).await;
        }

        async fn send_as(&self, sid: &str, to: SocketAddrV4, msg: BoardMsg, key: &Key) {
            self.socket
                .send_to(&encode_board(key, sid, &msg), to)
                .await
                .unwrap();
        }

        /// The next hub message and the satellite it is for.
        async fn recv_for(&self) -> Option<(String, HubMsg)> {
            let mut buf = [0u8; 2048];
            let (n, _) = tokio::time::timeout(WAIT, self.socket.recv_from(&mut buf))
                .await
                .ok()?
                .ok()?;
            decode_hub(&self.key, &buf[..n]).ok()
        }

        /// A phrase (0.8 s of speech, 0.6 s of silence) in window `session` of `sid`.
        async fn phrase(&self, sid: &str, to: SocketAddrV4, session: u32) {
            for seq in 1..=35u32 {
                let level = if seq <= 20 { 4_000 } else { 0 };
                let audio = BoardMsg::Audio {
                    session,
                    seq,
                    samples: vec![level; 640],
                };
                self.send_as(sid, to, audio, &self.key.clone()).await;
            }
        }
    }

    async fn start(
        with_key: bool,
    ) -> (Arc<Hub>, Arc<Mutex<Vec<Publish>>>, Board, tempfile::TempDir) {
        start_with(with_key, None, None).await
    }

    /// With a second satellite in the same room, and a stand-in for the assistant.
    async fn start_with(
        with_key: bool,
        second: Option<&str>,
        ask: Option<AskFn>,
    ) -> (Arc<Hub>, Arc<Mutex<Vec<Publish>>>, Board, tempfile::TempDir) {
        let (hub, log) = house().await;
        if let Some(sid) = second {
            add_satellite(&hub, sid).await;
        }
        let dir = tempfile::tempdir().unwrap();
        let stt = stt(dir.path(), with_key, "Jarvis, allume la lumière du salon.").await;
        let board = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let board_port = board.local_addr().unwrap().port();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let key = stt.key().unwrap_or_else(|| Key::from_bytes([1; 32]));
        let ask = ask.unwrap_or_else(default_ask);
        tokio::spawn(run_voice_with(
            Arc::clone(&hub),
            stt,
            socket,
            board_port,
            ask,
        ));
        (hub, log, Board { socket: board, key }, dir)
    }

    fn holds(log: &Mutex<Vec<Publish>>) -> Vec<serde_json::Value> {
        log.lock()
            .unwrap()
            .iter()
            .filter(|p| p.topic == HOLD_TOPIC)
            .map(|p| {
                assert!(!p.retain);
                serde_json::from_str(&p.payload).unwrap()
            })
            .collect()
    }

    async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let started = Instant::now();
        while !done() {
            assert!(started.elapsed() < WAIT, "never: {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn a_wake_word_turns_into_a_real_turn_and_a_reply() {
        let (hub, log, board, _dir) = start(true).await;
        let Some(HubMsg::Hello { nonce, hub: to, .. }) = board.recv().await else {
            panic!("no hello")
        };
        assert_eq!(to.ip(), &Ipv4Addr::LOCALHOST);
        let key = board.key.clone();
        let wake = BoardMsg::Wake {
            nonce,
            session: 1,
            kind: WakeKind::WakeWord,
            listen_ctr: 0,
        };
        board.send(to, wake, &key).await;
        for seq in 1..=35u32 {
            let level = if seq <= 20 { 4_000 } else { 0 };
            let audio = BoardMsg::Audio {
                session: 1,
                seq,
                samples: vec![level; 640],
            };
            board.send(to, audio, &key).await;
        }
        loop {
            match board.recv().await {
                Some(HubMsg::Stop { session, .. }) => {
                    assert_eq!(session, 1);
                    break;
                }
                Some(HubMsg::Hello { .. }) => continue,
                other => panic!("expected Stop, got {other:?}"),
            }
        }
        wait_for("the light command", || {
            log.lock()
                .unwrap()
                .iter()
                .any(|p| p.topic == LED_COMMAND_TOPIC)
        })
        .await;
        // No speech output on this hub: the reply stays in the history and the hold ends.
        wait_for("hold then release", || holds(&log).len() >= 2).await;
        let h = holds(&log);
        assert_eq!(
            h[0],
            json!({"id": SID, "players": [SID], "for_ms": HOLD_MS})
        );
        assert_eq!(h[1]["for_ms"], 0);
        let turns = hub.assist_turns(10).await.turns;
        assert_eq!(turns[0].client, format!("satellite:{SID}"));
        assert!(!turns[0].simulate);
        assert_eq!(turns[0].text, "allume la lumière du salon.");
    }

    #[tokio::test]
    async fn unsigned_wake_is_ignored() {
        let (hub, log, board, _dir) = start(true).await;
        let Some(HubMsg::Hello { nonce, hub: to, .. }) = board.recv().await else {
            panic!("no hello")
        };
        let wrong = Key::from_bytes([9; 32]);
        let wake = BoardMsg::Wake {
            nonce,
            session: 1,
            kind: WakeKind::WakeWord,
            listen_ctr: 0,
        };
        board.send(to, wake, &wrong).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(holds(&log).is_empty());
        assert!(hub.assist_turns(10).await.turns.is_empty());
    }

    #[tokio::test]
    async fn no_hello_without_a_key() {
        let (_hub, _log, board, _dir) = start(false).await;
        let mut buf = [0u8; 256];
        let got =
            tokio::time::timeout(Duration::from_millis(500), board.socket.recv_from(&mut buf))
                .await;
        assert!(got.is_err(), "the board heard something");
    }

    const SID2: &str = "abcdef012345";

    #[tokio::test]
    async fn a_refused_wake_is_stopped_at_once() {
        // Two satellites of one room hear the wake word: the second is told to close its
        // window at once (its speaker unmutes, its LED goes off), and gets no exchange.
        let (hub, log, board, _dir) = start_with(true, Some(SID2), None).await;
        let mut hellos = HashMap::new();
        while hellos.len() < 2 {
            if let Some((sid, HubMsg::Hello { nonce, hub: to, .. })) = board.recv_for().await {
                hellos.insert(sid, (nonce, to));
            }
        }
        let key = board.key.clone();
        let wake = |nonce| BoardMsg::Wake {
            nonce,
            session: 1,
            kind: WakeKind::WakeWord,
            listen_ctr: 0,
        };
        let (nonce_a, to) = hellos[SID];
        board.send_as(SID, to, wake(nonce_a), &key).await;
        wait_for("the first hold", || !holds(&log).is_empty()).await;
        let (nonce_b, to) = hellos[SID2];
        board.send_as(SID2, to, wake(nonce_b), &key).await;
        let started = Instant::now();
        loop {
            assert!(
                started.elapsed() < WAIT,
                "the second satellite was never stopped"
            );
            match board.recv_for().await {
                Some((sid, HubMsg::Stop { session: 1, .. })) if sid == SID2 => break,
                Some(_) => {}
                None => panic!("the second satellite was never stopped"),
            }
        }
        assert!(started.elapsed() < Duration::from_millis(500), "at once");
        assert!(
            holds(&log).iter().all(|h| h["id"] == SID),
            "no exchange for the second"
        );
        assert!(hub.assist_turns(10).await.turns.is_empty());
    }

    #[tokio::test]
    async fn a_question_reopens_the_window_after_it_is_spoken() {
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = Arc::clone(&asked);
        let ask: AskFn = Arc::new(move |_hub, _sid, _text| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async { (Some("clip-1".to_owned()), true) })
        });
        let (hub, log, board, _dir) = start_with(true, None, Some(ask)).await;
        let Some(HubMsg::Hello { nonce, hub: to, .. }) = board.recv().await else {
            panic!("no hello")
        };
        let key = board.key.clone();
        let wake = BoardMsg::Wake {
            nonce,
            session: 1,
            kind: WakeKind::WakeWord,
            listen_ctr: 0,
        };
        board.send(to, wake, &key).await;
        board.phrase(SID, to, 1).await;
        // The reply "plays": nidoo-sound reports it (again until the hub has the reply).
        let listen_ctr = loop {
            hub.ingest_message(
                crate::sound::SPOKEN_TOPIC,
                br#"{"id":"clip-1","played":true}"#,
                false,
            );
            match tokio::time::timeout(Duration::from_millis(500), board.recv_for()).await {
                Ok(Some((_, HubMsg::Listen { hub_ctr }))) => break hub_ctr,
                Ok(Some(_)) | Err(_) => {}
                Ok(None) => panic!("the board was never asked to listen"),
            }
        };
        let follow = BoardMsg::Wake {
            nonce,
            session: 2,
            kind: WakeKind::FollowUp,
            listen_ctr,
        };
        board.send(to, follow, &key).await;
        board.phrase(SID, to, 2).await;
        loop {
            match board.recv_for().await {
                Some((_, HubMsg::Stop { session: 2, .. })) => break,
                Some(_) => {}
                None => panic!("the follow-up was not heard"),
            }
        }
        wait_for("the answer asked", || {
            asked.load(std::sync::atomic::Ordering::SeqCst) == 2
        })
        .await;
        assert_eq!(
            holds(&log)[0]["for_ms"],
            HOLD_MS,
            "held through the question"
        );
        assert!(
            holds(&log).iter().all(|h| h["for_ms"] != 0),
            "not released between the question and the answer"
        );
    }
}
