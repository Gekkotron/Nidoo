//! A throwaway FTP server over a temp directory, for the backup store tests.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use libunftp::ServerBuilder;
use unftp_core::auth::{AuthenticationError, Authenticator, Credentials, Principal};
use unftp_sbe_fs::Filesystem;

/// Accepts exactly one user and password.
#[derive(Debug)]
struct OneLogin {
    user: String,
    password: String,
}

#[async_trait]
impl Authenticator for OneLogin {
    async fn authenticate(
        &self,
        username: &str,
        creds: &Credentials,
    ) -> Result<Principal, AuthenticationError> {
        if username == self.user && creds.password.as_deref() == Some(self.password.as_str()) {
            Ok(Principal {
                username: username.to_string(),
            })
        } else {
            Err(AuthenticationError::BadPassword)
        }
    }
}

/// Serves `dir` on a free local port and returns it. With `login` set, only that user and
/// password are accepted; without, anyone gets in. The server lives as long as the runtime.
pub async fn ftp_server(dir: &Path, login: Option<(&str, &str)>) -> u16 {
    serve(dir, login, None).await
}

/// [`ftp_server`] for anyone, closing a session left idle for `idle_secs`.
pub async fn ftp_server_idle(dir: &Path, idle_secs: u64) -> u16 {
    serve(dir, None, Some(idle_secs)).await
}

async fn serve(dir: &Path, login: Option<(&str, &str)>, idle_secs: Option<u64>) -> u16 {
    let port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let root = dir.to_path_buf();
    let sbe = Box::new(move || Filesystem::new(root.clone()).unwrap());
    let builder = match login {
        Some((user, password)) => ServerBuilder::with_authenticator(
            sbe,
            Arc::new(OneLogin {
                user: user.to_string(),
                password: password.to_string(),
            }),
        ),
        None => ServerBuilder::new(sbe),
    };
    // A range of its own per server keeps parallel tests off each other's data ports.
    let first = 40_000 + (port % 2_000) * 10;
    let mut builder = builder.passive_ports(first..=first + 9);
    if let Some(secs) = idle_secs {
        builder = builder.idle_session_timeout(secs);
    }
    let server = builder.build().expect("ftp server");
    tokio::spawn(async move {
        let _ = server.listen(format!("127.0.0.1:{port}")).await;
    });
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return port;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("ftp server did not start");
}
