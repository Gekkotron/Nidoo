//! Fetching the picture URL of a camera or image entity. The URL comes from the broker,
//! where any login may write it, so the fetch refuses the hub's own host (loopback),
//! link-local (cloud metadata), unspecified, multicast and broadcast addresses, dials the
//! address it checked, follows no redirect and keeps only `image/*` replies. The
//! configured Frigate origin is trusted as is: it may well be on `localhost`.

use std::net::{IpAddr, SocketAddr};

/// Where a picture fetch goes: the URL and, unless it is on the Frigate origin, the one
/// address it may connect to.
#[derive(Debug)]
pub struct Target {
    pub url: url::Url,
    pub pin: Option<SocketAddr>,
}

/// Whether the hub must never fetch a picture from `ip`.
pub fn blocked(ip: IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None if !v6.is_unspecified() && !v6.is_loopback() => {
                v6.to_ipv4().map_or(IpAddr::V6(v6), IpAddr::V4)
            }
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    };
    match ip {
        IpAddr::V4(v4) => {
            v4.octets()[0] == 0
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Whether `content_type` names an image.
pub fn is_image(content_type: &str) -> bool {
    content_type
        .trim()
        .get(..6)
        .is_some_and(|p| p.eq_ignore_ascii_case("image/"))
}

/// Checks `url` and resolves its host once: only `http` and `https`, and no address
/// [`blocked`] among the answers, unless the URL is on `frigate_origin`.
pub async fn target(url: &str, frigate_origin: Option<&str>) -> Result<Target, String> {
    let parsed = url::Url::parse(url).map_err(|err| err.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("scheme {} is not http or https", parsed.scheme()));
    }
    if frigate_origin.is_some() && crate::media::origin_of(url).as_deref() == frigate_origin {
        return Ok(Target {
            url: parsed,
            pin: None,
        });
    }
    let port = parsed.port_or_known_default().ok_or("no port")?;
    let addrs: Vec<SocketAddr> = match parsed.host().ok_or("no host")? {
        url::Host::Ipv4(v4) => vec![SocketAddr::new(v4.into(), port)],
        url::Host::Ipv6(v6) => vec![SocketAddr::new(v6.into(), port)],
        url::Host::Domain(name) => tokio::net::lookup_host((name, port))
            .await
            .map_err(|err| err.to_string())?
            .collect(),
    };
    if let Some(bad) = addrs.iter().find(|addr| blocked(addr.ip())) {
        return Err(format!("address {} is not allowed", bad.ip()));
    }
    let pin = *addrs.first().ok_or("host resolves to no address")?;
    Ok(Target {
        url: parsed,
        pin: Some(pin),
    })
}

/// Fetches the picture at `url` (see [`target`]): no proxy, no redirects, 10 s, 2xx, an
/// `image/*` reply of at most 5 MiB.
pub async fn fetch(
    url: &str,
    frigate_origin: Option<&str>,
) -> Result<(String, bytes::Bytes), String> {
    use crate::pictures::MAX_PICTURE_BYTES;
    let target = target(url, frigate_origin).await?;
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if let (Some(pin), Some(url::Host::Domain(name))) = (target.pin, target.url.host()) {
        builder = builder.resolve(name, pin);
    }
    let client = builder.build().map_err(|err| err.to_string())?;
    let mut response = client
        .get(target.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|err| err.to_string())?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .filter(|v| is_image(v))
        .ok_or("reply is not an image")?
        .to_owned();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| err.to_string())? {
        if body.len() + chunk.len() > MAX_PICTURE_BYTES {
            return Err("picture over 5 MiB".to_owned());
        }
        body.extend_from_slice(&chunk);
    }
    Ok((content_type, bytes::Bytes::from(body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("ip")
    }

    #[test]
    fn blocked_ranges() {
        for s in [
            "0.0.0.0",
            "0.1.2.3",
            "127.0.0.1",
            "127.255.0.1",
            "169.254.169.254",
            "224.0.0.1",
            "239.1.1.1",
            "255.255.255.255",
            "::",
            "::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
        ] {
            assert!(blocked(ip(s)), "{s} should be blocked");
        }
        for s in [
            "192.168.1.20",
            "10.0.0.5",
            "172.17.0.2",
            "8.8.8.8",
            "fd00::1",
            "2001:db8::1",
            "::ffff:192.168.1.20",
        ] {
            assert!(!blocked(ip(s)), "{s} should be allowed");
        }
    }

    #[test]
    fn is_image_cases() {
        for s in ["image/jpeg", "IMAGE/PNG; charset=x", " image/webp"] {
            assert!(is_image(s), "{s:?} is an image");
        }
        for s in ["application/json", "text/html", "", "imag"] {
            assert!(!is_image(s), "{s:?} is not an image");
        }
    }

    #[tokio::test]
    async fn target_refuses() {
        for url in [
            "ftp://192.168.1.20/a.jpg",
            "file:///etc/passwd",
            "http:///x",
            "http://127.0.0.1/a.jpg",
            "http://[::1]/a.jpg",
            "http://[::ffff:127.0.0.1]/a.jpg",
            "http://169.254.169.254/latest/meta-data",
            "http://localhost:5000/a.jpg",
            "nope",
        ] {
            assert!(target(url, None).await.is_err(), "{url} should be refused");
        }
    }

    #[tokio::test]
    async fn target_allows_lan() {
        let target = target("http://192.168.1.20:8080/a.jpg", None)
            .await
            .expect("LAN camera");
        assert_eq!(target.pin, Some("192.168.1.20:8080".parse().expect("addr")));
    }

    #[tokio::test]
    async fn target_allows_the_frigate_origin_on_loopback() {
        let url = "http://127.0.0.1:5000/api/x.jpg";
        let target = target(url, Some("http://127.0.0.1:5000"))
            .await
            .expect("Frigate origin");
        assert_eq!(target.pin, None);
        assert!(
            super::target(url, Some("http://127.0.0.1:5001"))
                .await
                .is_err()
        );
    }
}
