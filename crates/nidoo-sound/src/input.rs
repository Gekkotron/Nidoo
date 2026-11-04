//! The music input: raw s16le / 48 kHz / stereo PCM on a TCP port (the
//! phone app, reconnecting every 2 s while casting).
//!
//! One sender at a time: a new connection replaces the current one without
//! a stop/start, so a phone reconnecting after a Wi-Fi blip does not restart
//! the stream. The stream stops when the current connection closes or sends
//! nothing for `idle_stop`, and starts again with the next bytes.

use std::time::Duration;

use bytes::Bytes;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    Started,
    Pcm(Bytes),
    Stopped,
}

enum Step {
    Accepted(TcpStream, std::net::SocketAddr),
    AcceptFailed(std::io::Error),
    Read(std::io::Result<usize>),
    Idle,
}

/// Runs until `out` is closed.
pub async fn run_pcm_input(
    listener: TcpListener,
    idle_stop: Duration,
    out: mpsc::Sender<InputEvent>,
) -> anyhow::Result<()> {
    let mut current: Option<TcpStream> = None;
    let mut started = false;
    let mut last_byte = Instant::now();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let step = tokio::select! {
            a = listener.accept() => match a {
                Ok((s, peer)) => Step::Accepted(s, peer),
                Err(e) => Step::AcceptFailed(e),
            },
            r = read_current(&mut current, &mut buf) => Step::Read(r),
            _ = tokio::time::sleep_until(last_byte + idle_stop), if started => Step::Idle,
        };
        let event = match step {
            Step::Accepted(s, peer) => {
                tracing::info!(%peer, replaced = current.is_some(), "music input connected");
                current = Some(s);
                None
            }
            Step::AcceptFailed(e) => {
                tracing::warn!(error = %e, "music input accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                None
            }
            Step::Read(Ok(n)) if n > 0 => {
                last_byte = Instant::now();
                if !started {
                    started = true;
                    if out.send(InputEvent::Started).await.is_err() {
                        return Ok(());
                    }
                }
                Some(InputEvent::Pcm(Bytes::copy_from_slice(&buf[..n])))
            }
            Step::Read(r) => {
                if let Err(e) = r {
                    tracing::info!(error = %e, "music input connection failed");
                } else {
                    tracing::info!("music input disconnected");
                }
                current = None;
                started.then(|| {
                    started = false;
                    InputEvent::Stopped
                })
            }
            Step::Idle => {
                tracing::info!(?idle_stop, "music input idle");
                started = false;
                Some(InputEvent::Stopped)
            }
        };
        if let Some(event) = event
            && out.send(event).await.is_err()
        {
            return Ok(());
        }
    }
}

async fn read_current(current: &mut Option<TcpStream>, buf: &mut [u8]) -> std::io::Result<usize> {
    match current {
        Some(s) => s.read(buf).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    const IDLE: Duration = Duration::from_millis(300);
    const WAIT: Duration = Duration::from_secs(3);

    async fn start() -> (std::net::SocketAddr, mpsc::Receiver<InputEvent>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(run_pcm_input(listener, IDLE, tx));
        (addr, rx)
    }

    async fn next(rx: &mut mpsc::Receiver<InputEvent>) -> InputEvent {
        tokio::time::timeout(WAIT, rx.recv())
            .await
            .expect("no event")
            .expect("closed")
    }

    /// Next event that is not PCM.
    async fn next_control(rx: &mut mpsc::Receiver<InputEvent>) -> InputEvent {
        loop {
            match next(rx).await {
                InputEvent::Pcm(_) => continue,
                other => return other,
            }
        }
    }

    async fn quiet(rx: &mut mpsc::Receiver<InputEvent>, d: Duration) -> Vec<InputEvent> {
        let mut got = Vec::new();
        let deadline = tokio::time::Instant::now() + d;
        while let Ok(Some(e)) = tokio::time::timeout_at(deadline, rx.recv()).await {
            got.push(e);
        }
        got
    }

    #[tokio::test]
    async fn bytes_start_then_flow() {
        let (addr, mut rx) = start().await;
        let mut a = TcpStream::connect(addr).await.unwrap();
        a.write_all(&[1, 2, 3, 4]).await.unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::Started);
        assert_eq!(
            next(&mut rx).await,
            InputEvent::Pcm(bytes::Bytes::from_static(&[1, 2, 3, 4]))
        );
    }

    #[tokio::test]
    async fn idle_stops_and_bytes_restart() {
        let (addr, mut rx) = start().await;
        let mut a = TcpStream::connect(addr).await.unwrap();
        a.write_all(&[0; 64]).await.unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::Started);
        assert_eq!(next_control(&mut rx).await, InputEvent::Stopped);
        a.write_all(&[0; 64]).await.unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::Started);
    }

    #[tokio::test]
    async fn short_gaps_do_not_stop() {
        let (addr, mut rx) = start().await;
        let mut a = TcpStream::connect(addr).await.unwrap();
        for _ in 0..5 {
            a.write_all(&[0; 64]).await.unwrap();
            tokio::time::sleep(IDLE / 3).await;
        }
        let got = quiet(&mut rx, Duration::from_millis(10)).await;
        assert_eq!(got.iter().filter(|e| **e == InputEvent::Started).count(), 1);
        assert!(!got.contains(&InputEvent::Stopped));
    }

    #[tokio::test]
    async fn second_connection_replaces_first_without_stream_restart() {
        let (addr, mut rx) = start().await;
        let mut a = TcpStream::connect(addr).await.unwrap();
        a.write_all(&[1; 8]).await.unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::Started);
        let mut b = TcpStream::connect(addr).await.unwrap();
        // Give the input a moment to take B over before A writes again.
        tokio::time::sleep(Duration::from_millis(50)).await;
        b.write_all(&[2; 8]).await.unwrap();
        a.write_all(&[1; 8]).await.unwrap_or(());
        let got = quiet(&mut rx, IDLE / 2).await;
        assert!(!got.contains(&InputEvent::Stopped), "{got:?}");
        assert!(!got.contains(&InputEvent::Started), "{got:?}");
        let pcm: Vec<u8> = got
            .iter()
            .filter_map(|e| match e {
                InputEvent::Pcm(b) => Some(b.to_vec()),
                _ => None,
            })
            .flatten()
            .collect();
        assert!(
            pcm.starts_with(&[1; 8]) || pcm.iter().all(|&x| x == 2),
            "{pcm:?}"
        );
        assert!(pcm.ends_with(&[2; 8]), "B's bytes flow: {pcm:?}");
        assert!(!pcm[pcm.len() - 8..].contains(&1));
        // Closing the replaced connection changes nothing.
        drop(a);
        assert!(quiet(&mut rx, Duration::from_millis(100)).await.is_empty());
    }

    #[tokio::test]
    async fn close_stops_immediately() {
        let (addr, mut rx) = start().await;
        let mut a = TcpStream::connect(addr).await.unwrap();
        a.write_all(&[0; 8]).await.unwrap();
        assert_eq!(next(&mut rx).await, InputEvent::Started);
        drop(a);
        let started = tokio::time::Instant::now();
        assert_eq!(next_control(&mut rx).await, InputEvent::Stopped);
        assert!(
            started.elapsed() < IDLE,
            "stopped by the close, not the idle timer"
        );
    }

    #[tokio::test]
    async fn connection_without_bytes_starts_nothing() {
        let (addr, mut rx) = start().await;
        let a = TcpStream::connect(addr).await.unwrap();
        assert!(quiet(&mut rx, IDLE * 2).await.is_empty());
        drop(a);
        assert!(quiet(&mut rx, Duration::from_millis(100)).await.is_empty());
    }
}
