use super::*;

/// A WAV file as the hub writes it (`data` size given, or the streamed marker).
pub(crate) fn wav_with_size(rate: u32, channels: u16, samples: &[i16], data_size: u32) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + samples.len() as u32 * 2).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    v.extend_from_slice(&(channels * 2).to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_size.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

pub(crate) fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    wav_with_size(rate, channels, samples, samples.len() as u32 * 2)
}

fn frames(pcm: &[u8]) -> Vec<(i16, i16)> {
    pcm.as_chunks::<4>()
        .0
        .iter()
        .map(|f| {
            (
                i16::from_le_bytes([f[0], f[1]]),
                i16::from_le_bytes([f[2], f[3]]),
            )
        })
        .collect()
}

#[test]
fn parse_speak_reads_the_hub_message() {
    let msg = br#"{"id":"a","url":"/api/v1/assist/speech/a.wav","players":["afd56455c914"]}"#;
    assert_eq!(
        parse_speak(msg),
        Some(SpeakRequest {
            id: "a".into(),
            url: "/api/v1/assist/speech/a.wav".into(),
            players: vec!["afd56455c914".into()],
        })
    );
    assert_eq!(parse_speak(br#"{"id":"a","url":"/x","players":[]}"#), None);
    assert_eq!(parse_speak(b"nope"), None);
}

#[test]
fn parse_speak_takes_only_the_hubs_clips() {
    let with_url = |url: &str| {
        parse_speak(format!(r#"{{"id":"a","url":"{url}","players":["p"]}}"#).as_bytes())
    };
    assert!(with_url("/api/v1/assist/speech/0f3c-9a_b.wav").is_some());
    for url in [
        "/x",
        "/api/v1/settings",
        "/api/v1/assist/speech/",
        "/api/v1/assist/speech/../settings",
        "/api/v1/assist/speech/a/b.wav",
        "/api/v1/assist/speech/a.wav?x=1",
        "http://elsewhere/api/v1/assist/speech/a.wav",
        "/api/v1/assist/speech/..",
    ] {
        assert_eq!(with_url(url), None, "{url}");
    }
}

#[tokio::test]
async fn a_request_that_waited_too_long_is_not_fetched() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (requests, rx) = tokio::sync::mpsc::channel(4);
    let (to_group, mut spoken) = tokio::sync::mpsc::channel(4);
    let old = tokio::time::Instant::now() - MAX_WAIT - Duration::from_secs(1);
    requests.send((old, clip_request())).await.unwrap();
    drop(requests);
    let (publish, _published) = tokio::sync::mpsc::channel(4);
    run_speech_fetcher(format!("http://{addr}"), rx, to_group, publish).await;
    assert!(spoken.recv().await.is_none());
    let asked = tokio::time::timeout(Duration::from_millis(100), listener.accept()).await;
    assert!(asked.is_err(), "the hub was asked for the clip");
}

#[test]
fn mono_24k_is_doubled_and_duplicated() {
    let pcm = wav_to_stream_pcm(&wav(24_000, 1, &[0, 1000])).unwrap();
    assert_eq!(
        frames(&pcm),
        [(0, 0), (500, 500), (1000, 1000), (1000, 1000)]
    );
}

#[test]
fn stereo_48k_passes_through() {
    let pcm = wav_to_stream_pcm(&wav(48_000, 2, &[1, 2, 3, 4])).unwrap();
    assert_eq!(frames(&pcm), [(1, 2), (3, 4)]);
}

#[test]
fn streamed_header_reads_to_the_end() {
    let pcm = wav_to_stream_pcm(&wav_with_size(48_000, 1, &[5, 6, 7, 8], u32::MAX)).unwrap();
    assert_eq!(frames(&pcm), [(5, 5), (6, 6), (7, 7), (8, 8)]);
}

#[test]
fn rejects_non_pcm_and_8_bit() {
    let mut float = wav(48_000, 1, &[0, 0]);
    float[20] = 3; // format tag: IEEE float
    assert!(wav_to_stream_pcm(&float).is_err());
    let mut eight = wav(48_000, 1, &[0, 0]);
    eight[34] = 8; // bits per sample
    assert!(wav_to_stream_pcm(&eight).is_err());
    assert!(wav_to_stream_pcm(b"not a wav").is_err());
}

#[tokio::test]
async fn fetch_reads_a_clip_from_the_hub() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = wav(48_000, 2, &[9, 10]);
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let n = sock.read(&mut buf).await.unwrap();
        let head = String::from_utf8_lossy(&buf[..n]).to_string();
        assert!(
            head.starts_with("GET /api/v1/assist/speech/a.wav "),
            "{head}"
        );
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: audio/wav\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        sock.write_all(reply.as_bytes()).await.unwrap();
        sock.write_all(&body).await.unwrap();
    });
    let req = SpeakRequest {
        id: "a".into(),
        url: "/api/v1/assist/speech/a.wav".into(),
        players: vec!["p1".into()],
    };
    let speech = fetch_speech(&reqwest::Client::new(), &format!("http://{addr}"), &req)
        .await
        .unwrap();
    assert_eq!(speech.players, ["p1"]);
    assert_eq!(frames(&speech.pcm), [(9, 10)]);
}

/// A hub that sends the clip's header, then `pieces` pieces of it `gap` apart.
async fn slow_hub(pieces: usize, gap: Duration) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let samples: Vec<i16> = (0..2 * pieces as i16).collect();
    let body = wav_with_size(48_000, 2, &samples, u32::MAX);
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf).await.unwrap();
        let head = "HTTP/1.1 200 OK\r\ncontent-type: audio/wav\r\nconnection: close\r\n\r\n";
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(&body[..44]).await.unwrap();
        for piece in body[44..].chunks(4) {
            tokio::time::sleep(gap).await;
            if sock.write_all(piece).await.is_err() {
                return;
            }
        }
    });
    addr
}

fn clip_request() -> SpeakRequest {
    SpeakRequest {
        id: "a".into(),
        url: "/api/v1/assist/speech/a.wav".into(),
        players: vec!["p1".into()],
    }
}

#[tokio::test]
async fn a_clip_that_keeps_coming_is_read_past_the_stall_limit() {
    // Six pieces 100 ms apart: 600 ms in all, never 300 ms without data.
    let addr = slow_hub(6, Duration::from_millis(100)).await;
    let limits = FetchLimits {
        stall: Duration::from_millis(300),
        total: Duration::from_secs(5),
    };
    let speech = fetch_speech_with(
        &reqwest::Client::new(),
        &format!("http://{addr}"),
        &clip_request(),
        limits,
    )
    .await
    .unwrap();
    assert_eq!(frames(&speech.pcm).len(), 6);
}

#[tokio::test]
async fn a_stalled_clip_fails_at_the_stall_limit() {
    let addr = slow_hub(2, Duration::from_secs(2)).await;
    let limits = FetchLimits {
        stall: Duration::from_millis(200),
        total: Duration::from_secs(5),
    };
    let started = std::time::Instant::now();
    let got = fetch_speech_with(
        &reqwest::Client::new(),
        &format!("http://{addr}"),
        &clip_request(),
        limits,
    )
    .await;
    assert!(got.is_err());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_clip_slower_than_the_total_limit_fails() {
    let addr = slow_hub(20, Duration::from_millis(100)).await;
    let limits = FetchLimits {
        stall: Duration::from_millis(300),
        total: Duration::from_millis(500),
    };
    let got = fetch_speech_with(
        &reqwest::Client::new(),
        &format!("http://{addr}"),
        &clip_request(),
        limits,
    )
    .await;
    assert!(got.is_err());
}

#[tokio::test]
async fn failed_fetch_reports_not_played() {
    let (req_tx, req_rx) = tokio::sync::mpsc::channel(1);
    let (to_group, _group_rx) = tokio::sync::mpsc::channel(1);
    let (publish, mut published) = tokio::sync::mpsc::channel(4);
    tokio::spawn(run_speech_fetcher(
        "http://127.0.0.1:1".into(),
        req_rx,
        to_group,
        publish,
    ));
    let req = SpeakRequest {
        id: "gone".into(),
        url: "/api/v1/assist/speech/gone.wav".into(),
        players: vec!["p1".into()],
    };
    req_tx
        .send((tokio::time::Instant::now(), req))
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(15), published.recv())
        .await
        .unwrap();
    assert_eq!(
        got,
        Some(crate::group::Published::Spoken {
            id: "gone".into(),
            played: false
        })
    );
}
