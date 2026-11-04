//! Finds Sendspin players on the LAN: they advertise `_sendspin._tcp` over
//! mDNS (port + TXT `path`) and wait for the server to connect.

use std::net::Ipv4Addr;

use tokio::sync::mpsc;

/// The mDNS service type players advertise.
pub const SERVICE_TYPE: &str = "_sendspin._tcp.local.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// mDNS instance name, e.g. `nidoo-satellite`.
    pub instance: String,
    /// Where to connect, e.g. `ws://10.0.0.5:8928/sendspin`.
    pub url: url::Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Up(Discovered),
    Down { instance: String },
}

/// `ws://<ip>:<port><path>` for an advertised player; `None` for loopback or
/// unspecified addresses and paths that do not start with `/`.
pub fn url_from_service(ip: &str, port: u16, path: &str) -> Option<url::Url> {
    let ip: Ipv4Addr = ip.parse().ok()?;
    if ip.is_loopback() || ip.is_unspecified() || !path.starts_with('/') {
        return None;
    }
    url::Url::parse(&format!("ws://{ip}:{port}{path}")).ok()
}

/// The instance part of an mDNS full name.
pub fn instance_name(fullname: &str, service_type: &str) -> String {
    fullname
        .strip_suffix(service_type)
        .and_then(|s| s.strip_suffix('.'))
        .unwrap_or(fullname)
        .to_owned()
}

/// Browses for players until the returned receiver is dropped.
pub fn browse() -> anyhow::Result<mpsc::Receiver<DiscoveryEvent>> {
    let daemon = mdns_sd::ServiceDaemon::new()?;
    let events = daemon.browse(SERVICE_TYPE)?;
    let (tx, rx) = mpsc::channel(32);
    std::thread::Builder::new()
        .name("mdns-browse".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                let out = match event {
                    mdns_sd::ServiceEvent::ServiceResolved(s) => {
                        let instance = instance_name(&s.fullname, SERVICE_TYPE);
                        let path = s.get_property_val_str("path").unwrap_or_default();
                        let mut ips: Vec<Ipv4Addr> = s.get_addresses_v4().into_iter().collect();
                        ips.sort();
                        let url = ips
                            .iter()
                            .find_map(|ip| url_from_service(&ip.to_string(), s.port, path));
                        match url {
                            Some(url) => DiscoveryEvent::Up(Discovered { instance, url }),
                            None => {
                                tracing::debug!(%instance, path, "advertised player not usable");
                                continue;
                            }
                        }
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => DiscoveryEvent::Down {
                        instance: instance_name(&fullname, SERVICE_TYPE),
                    },
                    _ => continue,
                };
                if tx.blocking_send(out).is_err() {
                    break;
                }
            }
            let _ = daemon.shutdown();
        })?;
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_from_service_builds_ws_urls() {
        assert_eq!(
            url_from_service("10.0.0.5", 8928, "/sendspin")
                .unwrap()
                .as_str(),
            "ws://10.0.0.5:8928/sendspin"
        );
    }

    #[test]
    fn url_from_service_skips_loopback_unspecified_and_bad_paths() {
        assert!(url_from_service("127.0.0.1", 8928, "/sendspin").is_none());
        assert!(url_from_service("0.0.0.0", 8928, "/sendspin").is_none());
        assert!(url_from_service("10.0.0.5", 8928, "x").is_none());
        assert!(url_from_service("not-an-ip", 8928, "/sendspin").is_none());
    }

    #[test]
    fn instance_name_strips_the_service_type() {
        assert_eq!(
            instance_name(
                "nidoo-satellite._sendspin._tcp.local.",
                "_sendspin._tcp.local."
            ),
            "nidoo-satellite"
        );
        assert_eq!(instance_name("odd", "_sendspin._tcp.local."), "odd");
    }
}
