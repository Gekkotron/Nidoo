//! The line logged at startup that says which URL to open.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Interface names that belong to containers or virtual bridges, not to the LAN.
const SKIPPED_PREFIXES: [&str; 6] = ["docker", "br-", "veth", "cni", "flannel", "virbr"];

/// The IPv4 addresses a person on the LAN could reach the hub on, sorted and deduplicated.
pub fn lan_addresses(ifaces: &[(String, IpAddr)]) -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = ifaces
        .iter()
        .filter(|(name, _)| !SKIPPED_PREFIXES.iter().any(|p| name.starts_with(p)))
        .filter_map(|(_, ip)| match ip {
            IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() => Some(*v4),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// One line per address; `localhost` when there is none.
pub fn banner_lines(addrs: &[Ipv4Addr], port: u16, finished: bool) -> Vec<String> {
    let what = if finished { "open" } else { "set up" };
    let line = |host: String| format!("Open http://{host}:{port} to {what} your home");
    if addrs.is_empty() {
        return vec![line("localhost".to_string())];
    }
    addrs.iter().map(|a| line(a.to_string())).collect()
}

/// The addresses to print for a `bind` string: a specific IPv4 is shown as is, a wildcard
/// expands to the LAN addresses. `None` when `bind` has no parsable port.
fn addrs_for_bind(bind: &str, ifaces: &[(String, IpAddr)]) -> Option<(Vec<Ipv4Addr>, u16)> {
    let sock: SocketAddr = bind.parse().ok()?;
    let addrs = match sock.ip() {
        IpAddr::V4(v4) if !v4.is_unspecified() => vec![v4],
        _ => lan_addresses(ifaces),
    };
    Some((addrs, sock.port()))
}

/// Logs where to open the hub. Thin glue over the pure functions above.
pub fn log_banner(bind: &str, finished: bool) {
    let ifaces: Vec<(String, IpAddr)> = if_addrs::get_if_addrs()
        .map(|list| list.into_iter().map(|i| (i.name.clone(), i.ip())).collect())
        .unwrap_or_default();
    let Some((addrs, port)) = addrs_for_bind(bind, &ifaces) else {
        return;
    };
    for line in banner_lines(&addrs, port, finished) {
        tracing::info!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }
    fn iface(name: &str, addr: &str) -> (String, IpAddr) {
        (name.to_string(), addr.parse().unwrap())
    }

    #[test]
    fn skips_loopback_docker_and_ipv6() {
        let got = lan_addresses(&[
            iface("lo", "127.0.0.1"),
            iface("docker0", "172.17.0.1"),
            iface("eth0", "192.0.2.20"),
            iface("eth0", "fe80::1"),
        ]);
        assert_eq!(got, vec![ip("192.0.2.20")]);
    }

    #[test]
    fn skips_every_virtual_prefix_and_link_local() {
        let got = lan_addresses(&[
            iface("br-1a2b", "192.0.2.1"),
            iface("veth9f", "192.0.2.2"),
            iface("cni0", "192.0.2.3"),
            iface("flannel.1", "192.0.2.4"),
            iface("virbr0", "192.0.2.5"),
            iface("en0", "169.254.7.7"),
            iface("en0", "192.0.2.30"),
        ]);
        assert_eq!(got, vec![ip("192.0.2.30")]);
    }

    #[test]
    fn addresses_are_sorted_and_deduplicated() {
        let got = lan_addresses(&[
            iface("eth1", "192.0.2.50"),
            iface("eth0", "192.0.2.20"),
            iface("wlan0", "192.0.2.50"),
        ]);
        assert_eq!(got, vec![ip("192.0.2.20"), ip("192.0.2.50")]);
    }

    #[test]
    fn lines_say_set_up_until_finished() {
        assert_eq!(
            banner_lines(&[ip("192.0.2.20")], 8081, false),
            vec!["Open http://192.0.2.20:8081 to set up your home"]
        );
        assert_eq!(
            banner_lines(&[ip("192.0.2.20")], 8081, true),
            vec!["Open http://192.0.2.20:8081 to open your home"]
        );
    }

    #[test]
    fn one_line_per_address() {
        let lines = banner_lines(&[ip("192.0.2.20"), ip("192.0.2.21")], 80, false);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], "Open http://192.0.2.21:80 to set up your home");
    }

    #[test]
    fn no_address_falls_back_to_localhost() {
        assert_eq!(
            banner_lines(&[], 8081, false),
            vec!["Open http://localhost:8081 to set up your home"]
        );
        assert_eq!(
            banner_lines(&[], 8081, true),
            vec!["Open http://localhost:8081 to open your home"]
        );
    }

    #[test]
    fn wildcard_bind_uses_lan_addresses_and_specific_bind_is_shown_as_is() {
        let ifaces = [iface("eth0", "192.0.2.20")];
        assert_eq!(
            addrs_for_bind("0.0.0.0:8081", &ifaces),
            Some((vec![ip("192.0.2.20")], 8081))
        );
        assert_eq!(
            addrs_for_bind("192.0.2.99:9000", &ifaces),
            Some((vec![ip("192.0.2.99")], 9000))
        );
        assert_eq!(addrs_for_bind("not-an-address", &ifaces), None);
    }
}
