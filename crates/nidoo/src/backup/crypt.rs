//! The file pipeline: plaintext -> zstd (level 3) -> age (passphrase). `seal` is the writing
//! side, `open` the reading side. The result is a standard `.zst.age` file: `age -d`, `zstd -d`
//! and `tar x` recover it by hand.

use age::secrecy::SecretString;
use age::{DecryptError, Decryptor, Encryptor};
use async_compression::Level;
use async_compression::tokio::bufread::ZstdDecoder;
use async_compression::tokio::write::ZstdEncoder;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use tokio::io::{AsyncRead, AsyncWrite, BufReader, ReadBuf};
use tokio_util::compat::{
    Compat, FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt, TokioAsyncReadCompatExt,
    TokioAsyncWriteCompatExt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    WrongPassword,
    NotABackup,
    Io(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongPassword => f.write_str("Wrong password"),
            Self::NotABackup => f.write_str("This file is not a Nidoo backup"),
            Self::Io(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for OpenError {}

type Layer<W> = ZstdEncoder<Compat<age::stream::StreamWriter<Compat<W>>>>;
type Init<'a, W> = Pin<Box<dyn Future<Output = io::Result<Layer<W>>> + Send + 'a>>;

enum SealState<'a, W> {
    /// The age header is still to be written (it needs an async write).
    Init(Init<'a, W>),
    Ready(Layer<W>),
}

struct Sealer<'a, W>(SealState<'a, W>);

impl<W: AsyncWrite + Unpin + Send> Sealer<'_, W> {
    fn ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<&mut Layer<W>>> {
        if let SealState::Init(fut) = &mut self.0 {
            let layer = ready!(fut.as_mut().poll(cx))?;
            self.0 = SealState::Ready(layer);
        }
        match &mut self.0 {
            SealState::Ready(l) => Poll::Ready(Ok(l)),
            SealState::Init(_) => unreachable!("replaced just above"),
        }
    }
}

impl<W: AsyncWrite + Unpin + Send> AsyncWrite for Sealer<'_, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let l = ready!(self.ready(cx))?;
        Pin::new(l).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let l = ready!(self.ready(cx))?;
        Pin::new(l).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let l = ready!(self.ready(cx))?;
        Pin::new(l).poll_shutdown(cx)
    }
}

/// Wrap `out` so that plaintext written to it leaves as zstd-compressed, passphrase-encrypted
/// age data. `shutdown()` finalizes age's last chunk: a writer that is only dropped leaves a
/// file that fails to open.
pub fn seal<'a, W: AsyncWrite + Unpin + Send + 'a>(
    out: W,
    password: &str,
) -> impl AsyncWrite + Unpin + Send + 'a {
    let enc = Encryptor::with_user_passphrase(SecretString::from(password.to_owned()));
    let init: Init<'a, W> = Box::pin(async move {
        let w = enc.wrap_async_output(out.compat_write()).await?;
        Ok(ZstdEncoder::with_quality(
            w.compat_write(),
            Level::Precise(3),
        ))
    });
    Sealer(SealState::Init(init))
}

fn map_decrypt(e: DecryptError) -> OpenError {
    match e {
        DecryptError::DecryptionFailed | DecryptError::KeyDecryptionFailed => {
            OpenError::WrongPassword
        }
        DecryptError::Io(e) if e.kind() != io::ErrorKind::UnexpectedEof => {
            OpenError::Io(e.to_string())
        }
        _ => OpenError::NotABackup,
    }
}

/// Open a sealed stream. Fails at once on a wrong password or on a file that is not age data;
/// a file cut short fails later, when the read reaches the cut.
pub async fn open<R: AsyncRead + Unpin + Send>(
    input: R,
    password: &str,
) -> Result<impl AsyncRead + Unpin + Send, OpenError> {
    let decryptor = Decryptor::new_async(input.compat())
        .await
        .map_err(map_decrypt)?;
    let identity = age::scrypt::Identity::new(SecretString::from(password.to_owned()));
    let reader = decryptor
        .decrypt_async(std::iter::once(&identity as &dyn age::Identity))
        .map_err(map_decrypt)?;
    Ok(ErrorAfterData {
        inner: ZstdDecoder::new(BufReader::new(reader.compat())),
        pending: None,
    })
}

/// The zstd decoder can hand over its last bytes and fail in the same poll; tokio's own helpers
/// (and `debug_assert`s) expect an error to arrive with no data. Deliver the data first and the
/// error on the next read.
struct ErrorAfterData<R> {
    inner: R,
    pending: Option<io::Error>,
}

impl<R: AsyncRead + Unpin> AsyncRead for ErrorAfterData<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(e) = self.pending.take() {
            return Poll::Ready(Err(e));
        }
        let before = buf.filled().len();
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Err(e)) if buf.filled().len() > before => {
                self.pending = Some(e);
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::tar::{TarReader, TarWriter};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn random(n: usize) -> Vec<u8> {
        let mut v = vec![0u8; n];
        getrandom::getrandom(&mut v).unwrap();
        v
    }

    async fn sealed(data: &[u8], password: &str) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut w = seal(&mut out, password);
            w.write_all(data).await.unwrap();
            w.shutdown().await.unwrap();
        }
        out
    }

    #[tokio::test]
    async fn seal_then_open_round_trips() {
        let data = random(1024 * 1024);
        let bytes = sealed(&data, "correct horse").await;
        let mut r = open(&bytes[..], "correct horse").await.unwrap();
        let mut back = Vec::new();
        r.read_to_end(&mut back).await.unwrap();
        assert_eq!(back, data);
    }

    #[tokio::test]
    async fn wrong_password_is_reported() {
        let bytes = sealed(b"hello", "right password").await;
        let err = open(&bytes[..], "wrong password").await.err().unwrap();
        assert_eq!(err, OpenError::WrongPassword);
        assert_eq!(err.to_string(), "Wrong password");
    }

    #[tokio::test]
    async fn garbage_is_not_a_backup() {
        let err = open(&random(4096)[..], "whatever1").await.err().unwrap();
        assert_eq!(err, OpenError::NotABackup);
        assert_eq!(err.to_string(), "This file is not a Nidoo backup");
        let err = open(&b""[..], "whatever1").await.err().unwrap();
        assert_eq!(err, OpenError::NotABackup);
    }

    #[tokio::test]
    async fn truncated_ciphertext_fails() {
        let bytes = sealed(&random(300_000), "pw-pw-pw-pw").await;
        let cut = &bytes[..bytes.len() - 100];
        let mut r = open(cut, "pw-pw-pw-pw").await.unwrap();
        let mut back = Vec::new();
        assert!(r.read_to_end(&mut back).await.is_err());
    }

    #[tokio::test]
    async fn sealed_file_opens_with_standard_tools() {
        let mut out = Vec::new();
        let mut tw = TarWriter::new(seal(&mut out, "hand recovery"));
        tw.entry("settings.json", b"{\"k\":1}").await.unwrap();
        tw.entry("db/t/000001.copy", &[9u8; 2000]).await.unwrap();
        let mut w = tw.finish().await.unwrap();
        w.shutdown().await.unwrap();
        drop(w);

        let decryptor = age::Decryptor::new(&out[..]).unwrap();
        let identity = age::scrypt::Identity::new(SecretString::from("hand recovery".to_owned()));
        let plain = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .unwrap();
        let zst = zstd::stream::read::Decoder::new(plain).unwrap();
        let mut archive = tar::Archive::new(zst);
        let names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["settings.json", "db/t/000001.copy"]);

        // And the async reader agrees.
        let r = open(&out[..], "hand recovery").await.unwrap();
        let mut tr = TarReader::new(r);
        assert_eq!(tr.next().await.unwrap().unwrap().0, "settings.json");
    }
}
