//! A minimal async tar writer and reader: just enough to carry whole entries (a name and its
//! bytes) through an `AsyncWrite` / `AsyncRead`. The output is plain GNU tar, so `tar x` and the
//! `tar` crate open it.

use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// The largest entry the reader accepts (and the writer's callers cut their data to).
pub const MAX_ENTRY: usize = 8 * 1024 * 1024;

const BLOCK: usize = 512;

fn pad_len(size: usize) -> usize {
    (BLOCK - size % BLOCK) % BLOCK
}

pub struct TarWriter<W: AsyncWrite + Unpin> {
    w: W,
}

impl<W: AsyncWrite + Unpin> TarWriter<W> {
    pub fn new(w: W) -> Self {
        Self { w }
    }

    /// Append one regular file (mode 0600, modified now).
    pub async fn entry(&mut self, path: &str, data: &[u8]) -> io::Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_path(path)?;
        header.set_size(data.len() as u64);
        header.set_mode(0o600);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        header.set_mtime(now);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        self.w.write_all(header.as_bytes()).await?;
        self.w.write_all(data).await?;
        self.w.write_all(&[0u8; BLOCK][..pad_len(data.len())]).await
    }

    /// Write the two closing zero blocks, flush, and hand the writer back.
    pub async fn finish(mut self) -> io::Result<W> {
        self.w.write_all(&[0u8; 2 * BLOCK]).await?;
        self.w.flush().await?;
        Ok(self.w)
    }
}

pub struct TarReader<R: AsyncRead + Unpin> {
    r: R,
}

impl<R: AsyncRead + Unpin> TarReader<R> {
    pub fn new(r: R) -> Self {
        Self { r }
    }

    /// The next entry, or `None` at the closing zero block. A stream that ends before it, a bad
    /// header checksum or an entry over [`MAX_ENTRY`] is an error.
    pub async fn next(&mut self) -> io::Result<Option<(String, Vec<u8>)>> {
        let mut block = [0u8; BLOCK];
        self.r.read_exact(&mut block).await?;
        if block.iter().all(|b| *b == 0) {
            return Ok(None);
        }
        let header = tar::Header::from_byte_slice(&block);
        let stored = header.cksum()?;
        let computed = block
            .iter()
            .enumerate()
            .map(|(i, b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    u32::from(*b)
                }
            })
            .sum::<u32>();
        if stored != computed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad tar header checksum",
            ));
        }
        let size = header.entry_size()?;
        if size > MAX_ENTRY as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "tar entry too large",
            ));
        }
        let size = size as usize;
        let path = String::from_utf8(header.path_bytes().into_owned())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "tar path is not UTF-8"))?;
        let mut data = vec![0u8; size];
        self.r.read_exact(&mut data).await?;
        let mut pad = [0u8; BLOCK];
        self.r.read_exact(&mut pad[..pad_len(size)]).await?;
        Ok(Some((path, data)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn write_all(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = TarWriter::new(Vec::new());
        for (p, d) in entries {
            w.entry(p, d).await.unwrap();
        }
        w.finish().await.unwrap()
    }

    #[tokio::test]
    async fn tar_round_trips_entries() {
        let entries = vec![
            ("manifest.json", b"{\"a\":1}".to_vec()),
            ("db/empty/000001.copy", Vec::new()),
            ("db/big/000001.copy", vec![7u8; MAX_ENTRY]),
        ];
        let bytes = write_all(&entries).await;
        let mut r = TarReader::new(&bytes[..]);
        for (p, d) in &entries {
            let (path, data) = r.next().await.unwrap().unwrap();
            assert_eq!(&path, p);
            assert_eq!(&data, d);
        }
        assert!(r.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn tar_output_opens_with_the_tar_crate() {
        let entries = vec![
            ("settings.json", b"{}".to_vec()),
            ("db/t/000001.copy", vec![1u8; 1000]),
        ];
        let bytes = write_all(&entries).await;
        let mut archive = tar::Archive::new(&bytes[..]);
        let mut seen = Vec::new();
        for e in archive.entries().unwrap() {
            let mut e = e.unwrap();
            let path = e.path().unwrap().to_string_lossy().into_owned();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut e, &mut data).unwrap();
            seen.push((path, data));
        }
        let expected: Vec<_> = entries
            .into_iter()
            .map(|(p, d)| (p.to_string(), d))
            .collect();
        assert_eq!(seen, expected);
    }

    #[tokio::test]
    async fn oversized_entry_is_an_error() {
        let bytes = write_all(&[("x", vec![0u8; MAX_ENTRY + 1])]).await;
        let mut r = TarReader::new(&bytes[..]);
        assert!(r.next().await.is_err());
    }
}
