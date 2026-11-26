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

async fn start(with_key: bool) -> (Arc<Hub>, Arc<Mutex<Vec<Publish>>>, Board, tempfile::TempDir) {
    start_with(with_key, None, None).await
}

/// With a second satellite in the same room, and a stand-in for the assistant.
async fn start_with(
    with_key: bool,
    second: Option<&str>,
    ask: Option<AskFn>,
) -> (Arc<Hub>, Arc<Mutex<Vec<Publish>>>, Board, tempfile::TempDir) {
    start_heard(with_key, second, ask, "Jarvis, allume la lumière du salon.").await
}

/// [`start_with`], the engines hearing `heard`.
async fn start_heard(
    with_key: bool,
    second: Option<&str>,
    ask: Option<AskFn>,
    heard: &'static str,
) -> (Arc<Hub>, Arc<Mutex<Vec<Publish>>>, Board, tempfile::TempDir) {
    let (hub, log) = house().await;
    if let Some(sid) = second {
        add_satellite(&hub, sid).await;
    }
    let dir = tempfile::tempdir().unwrap();
    let stt = stt(dir.path(), with_key, heard).await;
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
        tokio::time::timeout(Duration::from_millis(500), board.socket.recv_from(&mut buf)).await;
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

/// The hub speaks through a tone synthesiser (so `speak_clip` works).
async fn with_speech(hub: &Hub, dir: &Path) {
    use crate::assist::tts::tone::ToneLoader;
    use crate::assist::tts::{Tts, TtsConfig};
    use axum::body::Body;
    use nidoo_api::LanguageRequest;
    let tts = Tts::start(TtsConfig {
        root: dir.join("tts"),
        hf_base: "http://127.0.0.1:1".to_owned(),
        loader: Arc::new(ToneLoader),
    });
    let upload = tts.upload_open().unwrap();
    let files: [(&str, &[u8]); 3] = [
        (
            "config.json",
            br#"{"languages":["fr","en"],"model_id":"M"}"#,
        ),
        ("tokenizer.json", b"{}"),
        ("model.q8.gguf", b"weights"),
    ];
    for (name, body) in files {
        tts.upload_file(&upload.id, name, Body::from(body.to_vec()))
            .await
            .unwrap();
    }
    let model = tts.upload_finish(&upload.id).unwrap();
    let req = LanguageRequest {
        model: model.id,
        voice: None,
    };
    tts.set_language("fr", req).await.unwrap();
    for _ in 0..250 {
        let status = serde_json::to_value(tts.status()).unwrap();
        if status["languages"][0]["engine"]["state"] == "ready" {
            hub.set_tts(tts);
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the synthesiser never got ready");
}

fn speaks(log: &Mutex<Vec<Publish>>) -> Vec<serde_json::Value> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|p| p.topic == crate::sound::SPEAK_TOPIC)
        .map(|p| serde_json::from_str(&p.payload).unwrap())
        .collect()
}

/// Reports the `n`th spoken clip as played until the board is asked to listen.
async fn play_and_wait_listen(
    hub: &Hub,
    log: &Mutex<Vec<Publish>>,
    board: &Board,
    n: usize,
) -> u64 {
    wait_for("the question spoken", || speaks(log).len() >= n).await;
    let clip = speaks(log)[n - 1]["id"].as_str().unwrap().to_owned();
    loop {
        let report = json!({"id": clip, "played": true}).to_string();
        hub.ingest_message(crate::sound::SPOKEN_TOPIC, report.as_bytes(), false);
        match tokio::time::timeout(Duration::from_millis(500), board.recv_for()).await {
            Ok(Some((_, HubMsg::Listen { hub_ctr }))) => return hub_ctr,
            Ok(Some(_)) | Err(_) => {}
            Ok(None) => panic!("the board was never asked to listen"),
        }
    }
}

use super::super::ask::AskOutcome;

#[tokio::test]
async fn ask_aloud_hears_the_answer() {
    let (hub, log, board, dir) = start_heard(true, None, None, "oui").await;
    with_speech(&hub, dir.path()).await;
    let Some(HubMsg::Hello { nonce, hub: to, .. }) = board.recv().await else {
        panic!("no hello")
    };
    let key = board.key.clone();
    let asker = Arc::clone(&hub);
    let asked = tokio::spawn(async move {
        asker
            .ask_aloud(SpeakTarget::Room("living".into()), "Je chauffe ?")
            .await
    });
    let listen_ctr = play_and_wait_listen(&hub, &log, &board, 1).await;
    assert_eq!(holds(&log)[0]["for_ms"], HOLD_MS, "held for the question");
    let follow = BoardMsg::Wake {
        nonce,
        session: 1,
        kind: WakeKind::FollowUp,
        listen_ctr,
    };
    board.send(to, follow, &key).await;
    board.phrase(SID, to, 1).await;
    let outcome = tokio::time::timeout(WAIT, asked).await.unwrap().unwrap();
    assert_eq!(
        outcome,
        AskOutcome::Heard {
            sid: SID.to_owned(),
            text: "oui".to_owned()
        }
    );
    wait_for("the release", || {
        holds(&log).last().is_some_and(|h| h["for_ms"] == 0)
    })
    .await;
    assert!(
        hub.assist_turns(10).await.turns.is_empty(),
        "the answer is not an assistant turn"
    );
}

#[tokio::test]
async fn busy_satellites_are_skipped() {
    let (hub, log, board, _dir) = start(true).await;
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
    wait_for("the exchange's hold", || !holds(&log).is_empty()).await;
    let outcome = hub.ask_aloud(SpeakTarget::All, "Je chauffe ?").await;
    assert_eq!(
        outcome,
        AskOutcome::NoSatellite("no free satellite".to_owned())
    );
    assert!(speaks(&log).is_empty(), "nothing was spoken");
}

#[tokio::test]
async fn ask_aloud_without_voice_is_off() {
    let (hub, _log) = house().await;
    assert_eq!(
        hub.ask_aloud(SpeakTarget::All, "Je chauffe ?").await,
        AskOutcome::Off
    );
}

#[tokio::test]
async fn an_unknown_room_has_no_satellite() {
    let (hub, _log, board, _dir) = start(true).await;
    board.recv().await.expect("hello");
    let outcome = hub
        .ask_aloud(SpeakTarget::Room("nowhere".into()), "Je chauffe ?")
        .await;
    assert!(matches!(outcome, AskOutcome::NoSatellite(_)), "{outcome:?}");
}

#[tokio::test]
async fn closed_asker_releases_the_satellite() {
    let (hub, log, board, dir) = start_heard(true, None, None, "oui").await;
    with_speech(&hub, dir.path()).await;
    board.recv().await.expect("hello");
    let asker = Arc::clone(&hub);
    let asked =
        tokio::spawn(async move { asker.ask_aloud(SpeakTarget::All, "Je chauffe ?").await });
    play_and_wait_listen(&hub, &log, &board, 1).await;
    assert!(holds(&log).iter().all(|h| h["for_ms"] != 0));
    // The flow run is cancelled: the future is dropped mid-question.
    asked.abort();
    wait_for("the release", || {
        holds(&log).last().is_some_and(|h| h["for_ms"] == 0)
    })
    .await;
    // The satellite is free again: a new question is spoken.
    let asker = Arc::clone(&hub);
    let again = tokio::spawn(async move { asker.ask_aloud(SpeakTarget::All, "Encore ?").await });
    wait_for("the second question", || speaks(&log).len() == 2).await;
    again.abort();
}

#[tokio::test]
async fn a_closed_asker_is_not_spoken_to() {
    let (hub, log, board, dir) = start_heard(true, None, None, "oui").await;
    with_speech(&hub, dir.path()).await;
    board.recv().await.expect("hello");
    let (reply, answer) = oneshot::channel();
    drop(answer);
    hub.voice_asks
        .get()
        .expect("voice service")
        .send(AskRequest {
            target: SpeakTarget::All,
            text: "Je chauffe ?".to_owned(),
            reply,
        })
        .await
        .expect("sent");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(speaks(&log).is_empty(), "nothing was spoken");
    assert!(holds(&log).is_empty(), "nothing was held");
}

#[tokio::test]
async fn busy_room_is_named_in_the_detail() {
    let (hub, log, board, _dir) = start(true).await;
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
    wait_for("the exchange's hold", || !holds(&log).is_empty()).await;
    let outcome = hub
        .ask_aloud(SpeakTarget::Room("living".into()), "Je chauffe ?")
        .await;
    assert_eq!(
        outcome,
        AskOutcome::NoSatellite("no free satellite in living".to_owned())
    );
}

#[tokio::test]
async fn a_late_clip_of_a_cancelled_ask_is_dropped() {
    let (hub, _log) = house().await;
    let dir = tempfile::tempdir().unwrap();
    let stt = stt(dir.path(), true, "oui").await;
    let board = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (events, _results) = mpsc::channel(8);
    let mut voice = Voice {
        hub,
        stt,
        socket: Arc::new(socket),
        board_port: board.local_addr().unwrap().port(),
        sats: HashMap::new(),
        rooms: Rooms::default(),
        counter: HubCounter::default(),
        started: Instant::now(),
        events,
        refused: 0,
        ask: default_ask(),
        pending: HashMap::new(),
        next_ask: 0,
    };
    voice.greet().await;
    // Ask 2 now runs on the satellite; the clip of cancelled ask 1 arrives late.
    voice.sats.get_mut(SID).expect("satellite").ask_id = Some(2);
    voice.feed(SID, Input::AskStart).await;
    let clip = |id| Event::AskClip {
        sid: SID.to_owned(),
        id,
        clip: Some("late".to_owned()),
    };
    voice.on_event(clip(1)).await;
    assert!(!voice.sats[SID].session.is_speaking(), "stale clip taken");
    voice.on_event(clip(2)).await;
    assert!(voice.sats[SID].session.is_speaking(), "own clip dropped");
}

#[tokio::test]
async fn silence_answers_silence() {
    let (hub, log, board, dir) = start_heard(true, None, None, "oui").await;
    with_speech(&hub, dir.path()).await;
    board.recv().await.expect("hello");
    let asker = Arc::clone(&hub);
    let asked =
        tokio::spawn(async move { asker.ask_aloud(SpeakTarget::All, "Je chauffe ?").await });
    play_and_wait_listen(&hub, &log, &board, 1).await;
    let outcome = tokio::time::timeout(WAIT, asked).await.unwrap().unwrap();
    assert_eq!(outcome, AskOutcome::Silence);
    wait_for("the release", || {
        holds(&log).last().is_some_and(|h| h["for_ms"] == 0)
    })
    .await;
}
