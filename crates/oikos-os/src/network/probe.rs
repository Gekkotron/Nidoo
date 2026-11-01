//! The ICMP prober: one echo request per probe, over an unprivileged datagram socket when the
//! system allows it and a raw socket otherwise.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use socket2::Type;
use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence, SurgeError};

use super::tracker::Probe;

/// Sends one ping to `host` and reports what came back.
pub trait Prober: Send + Sync + 'static {
    fn probe<'a>(&'a self, host: &'a str, timeout: Duration) -> BoxFuture<'a, Probe>;
}

/// The real prober. Each IP family opens its socket on first use; a family that cannot be
/// opened only fails the hosts that resolve to it.
pub struct IcmpProber {
    v4: Mutex<Option<Result<Client, String>>>,
    v6: Mutex<Option<Result<Client, String>>>,
    counter: AtomicU16,
}

impl Default for IcmpProber {
    fn default() -> Self {
        Self::new()
    }
}

impl IcmpProber {
    pub fn new() -> IcmpProber {
        let mut seed = [0u8; 2];
        let _ = getrandom::getrandom(&mut seed);
        IcmpProber {
            v4: Mutex::new(None),
            v6: Mutex::new(None),
            counter: AtomicU16::new(u16::from_le_bytes(seed)),
        }
    }

    fn client(&self, v6: bool) -> Result<Client, String> {
        let slot = if v6 { &self.v6 } else { &self.v4 };
        let mut slot = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        slot.get_or_insert_with(|| open_client(v6)).clone()
    }

    async fn ping(&self, host: &str) -> Probe {
        let Some(addr) = resolve(host).await else {
            return Probe::Miss;
        };
        let client = match self.client(addr.is_ipv6()) {
            Ok(client) => client,
            Err(reason) => return Probe::CannotPing(reason),
        };
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let mut pinger = client.pinger(addr, PingIdentifier(n)).await;
        match pinger.ping(PingSequence(n), &[0; 8]).await {
            Ok((_, rtt)) => Probe::Reply(rtt),
            Err(SurgeError::IOError(err)) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                Probe::CannotPing(err.to_string())
            }
            Err(_) => Probe::Miss,
        }
    }
}

impl Prober for IcmpProber {
    fn probe<'a>(&'a self, host: &'a str, timeout: Duration) -> BoxFuture<'a, Probe> {
        Box::pin(async move {
            tokio::time::timeout(timeout, self.ping(host))
                .await
                .unwrap_or(Probe::Miss)
        })
    }
}

/// The first IPv4 address of `host`, else the first IPv6 one; `None` when it does not resolve.
async fn resolve(host: &str) -> Option<IpAddr> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 0)).await.ok()?.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .map(SocketAddr::ip)
}

/// Opens the ICMP client of one family: datagram socket first, raw socket as the fallback.
fn open_client(v6: bool) -> Result<Client, String> {
    let kind = if v6 { ICMP::V6 } else { ICMP::V4 };
    let open = |sock_type| {
        Client::new(
            &Config::builder()
                .kind(kind)
                .sock_type_hint(sock_type)
                .build(),
        )
    };
    open(Type::DGRAM)
        .or_else(|_| open(Type::RAW))
        .map_err(|err| format!("cannot open an ICMP socket: {err}"))
}

/// The prober the hub uses.
pub fn default_prober() -> Arc<dyn Prober> {
    Arc::new(IcmpProber::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unresolvable_host_is_a_miss() {
        let prober = IcmpProber::new();
        let probe = prober
            .probe("does-not-resolve.invalid", Duration::from_secs(2))
            .await;
        assert_eq!(probe, Probe::Miss);
    }
}
