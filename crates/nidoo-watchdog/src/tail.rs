//! Follows a growing log file, like `tail -F` from the end.

use std::io::SeekFrom;
use std::path::PathBuf;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const POLL: Duration = Duration::from_secs(1);

#[cfg(unix)]
fn file_id(md: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(md.ino())
}

#[cfg(not(unix))]
fn file_id(_: &std::fs::Metadata) -> Option<u64> {
    None
}

/// Sends each line appended to `path` (without its `\n`) to `lines`, polling every second.
///
/// It starts at the end of the file as it is when this function returns, so history does not
/// replay as alerts. When the file shrinks or is replaced (rotation) it reads the new file from
/// the start, and a file that disappears is picked up again from the start when it returns. A
/// half-written line waits for its newline. The task ends when `lines` is closed.
pub fn tail(path: PathBuf, lines: mpsc::Sender<String>) -> JoinHandle<()> {
    // Look at the file now, so that what is appended after this call is never skipped.
    let start = std::fs::metadata(&path).ok();
    tokio::spawn(async move {
        let mut seen = start.is_some();
        let mut inode = start.as_ref().and_then(file_id);
        let mut pos = start.as_ref().map_or(0, |md| md.len());
        let mut gone = false;
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let md = match tokio::fs::metadata(&path).await {
                Ok(md) => md,
                Err(_) => {
                    gone = true;
                    tokio::time::sleep(POLL).await;
                    continue;
                }
            };
            let id = file_id(&md);
            if !seen {
                // First sight of a file that did not exist at start: skip what it holds.
                seen = true;
                inode = id;
                pos = md.len();
            } else if gone || id != inode || md.len() < pos {
                tracing::info!(path = %path.display(), "log file rotated or reopened, reading from the start");
                inode = id;
                pos = 0;
                pending.clear();
            }
            gone = false;
            if md.len() > pos {
                match read_from(&path, pos).await {
                    Ok(bytes) => {
                        pos += bytes.len() as u64;
                        pending.extend_from_slice(&bytes);
                        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                            let line: Vec<u8> = pending.drain(..=end).collect();
                            let line = String::from_utf8_lossy(&line[..end]).into_owned();
                            if lines.send(line).await.is_err() {
                                return;
                            }
                        }
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "cannot read the log file")
                    }
                }
            }
            tokio::time::sleep(POLL).await;
        }
    })
}

async fn read_from(path: &PathBuf, pos: u64) -> std::io::Result<Vec<u8>> {
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(SeekFrom::Start(pos)).await?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).await?;
    Ok(bytes)
}
