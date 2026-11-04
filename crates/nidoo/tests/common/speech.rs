//! A ready fake speech engine and speaker satellites, for the speak tests.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nidoo::assist::tts::tone::ToneLoader;
use nidoo::assist::tts::{Tts, TtsConfig};
use nidoo::hub::Hub;
use nidoo_api::{EngineState, LanguageRequest};
use serde_json::json;

/// A speech facade on the tone synthesiser with a French model loaded and ready.
pub async fn ready_tts(root: PathBuf) -> Arc<Tts> {
    let tts = Tts::start(TtsConfig {
        root,
        hf_base: "http://127.0.0.1:1".to_owned(),
        loader: Arc::new(ToneLoader),
    });
    let session = tts.upload_open().expect("upload session");
    let files: [(&str, &[u8]); 3] = [
        ("config.json", br#"{"languages":["fr"],"model_id":"M"}"#),
        ("tokenizer.json", b"{}"),
        ("model.q8.gguf", b"weights"),
    ];
    for (name, body) in files {
        tts.upload_file(&session.id, name, axum::body::Body::from(body.to_vec()))
            .await
            .expect("upload file");
    }
    let model = tts.upload_finish(&session.id).expect("finish upload");
    tts.set_language(
        "fr",
        LanguageRequest {
            model: model.id,
            voice: None,
        },
    )
    .await
    .expect("set language");
    let end = Instant::now() + Duration::from_secs(5);
    while !matches!(
        tts.status().languages.first().map(|l| &l.engine),
        Some(EngineState::Ready { .. })
    ) {
        assert!(Instant::now() < end, "speech never ready");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tts
}

/// A satellite as `nidoo-sound` announces it, allowed and connected, in `area`.
pub fn satellite(hub: &Hub, sid: &str, area: &str) {
    let device = format!("nidoo_sound_{sid}");
    for (component, object) in [
        ("number", "volume"),
        ("switch", "mute"),
        ("switch", "allowed"),
        ("binary_sensor", "connected"),
    ] {
        let mut config = json!({
            "name": object,
            "unique_id": format!("{device}_{object}"),
            "state_topic": format!("nidoo-sound/{sid}/{object}"),
            "payload_on": "ON",
            "payload_off": "OFF",
            "device": { "identifiers": [device], "name": sid },
        });
        if component != "binary_sensor" {
            config["command_topic"] = json!(format!("nidoo-sound/{sid}/{object}/set"));
        }
        hub.ingest(
            &format!("nidoo/{component}/{device}/{object}/config"),
            config.to_string().as_bytes(),
        );
        hub.ingest(
            &format!("nidoo/ui/{device}_{object}"),
            json!({ "area": area }).to_string().as_bytes(),
        );
    }
    hub.ingest(&format!("nidoo-sound/{sid}/allowed"), b"ON");
    hub.ingest(&format!("nidoo-sound/{sid}/connected"), b"ON");
}
