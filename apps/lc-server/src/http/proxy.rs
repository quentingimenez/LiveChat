//! Client IP announced by a trusted reverse proxy.
//!
//! Only read when the operator opts in (`TRUST_PROXY_HEADERS` for the rate
//! limit, `ADMIN_TRUST_FORWARDED_FOR` for the admin local-only check).
//!
//! The first `X-Forwarded-For` entries are written by the client: anyone can
//! send `X-Forwarded-For: 127.0.0.1` or a new address per request, and every
//! proxy *appends* to it. So the address is taken, in order, from:
//!
//! 1. `X-Real-IP`, which the proxy sets itself (Nginx Proxy Manager sets it to
//!    the client address, after `real_ip_header CF-Connecting-IP` behind
//!    Cloudflare);
//! 2. the **last** `X-Forwarded-For` entry, appended by the proxy closest to
//!    the server.

use std::net::IpAddr;

use axum::http::HeaderMap;

/// Client address from the proxy headers, or `None` when absent or invalid.
pub fn proxied_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let real_ip = headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<IpAddr>().ok());
    real_ip.or_else(|| {
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit(',').next())
            .and_then(|hop| hop.trim().parse::<IpAddr>().ok())
    })
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn real_ip_wins_over_a_forged_forwarded_for() {
        let map = headers(&[
            ("x-forwarded-for", "127.0.0.1, 203.0.113.7"),
            ("x-real-ip", "203.0.113.7"),
        ]);
        assert_eq!(
            proxied_client_ip(&map),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)))
        );
    }

    #[test]
    fn falls_back_to_the_hop_appended_by_the_proxy() {
        let map = headers(&[("x-forwarded-for", "127.0.0.1, 198.51.100.4")]);
        assert_eq!(
            proxied_client_ip(&map),
            Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4)))
        );
    }

    #[test]
    fn invalid_real_ip_falls_back_to_forwarded_for() {
        let map = headers(&[
            ("x-real-ip", "unknown"),
            ("x-forwarded-for", "198.51.100.4"),
        ]);
        assert_eq!(
            proxied_client_ip(&map),
            Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4)))
        );
    }

    #[test]
    fn missing_or_garbage_headers_give_none() {
        assert_eq!(proxied_client_ip(&HeaderMap::new()), None);
        assert_eq!(
            proxied_client_ip(&headers(&[("x-forwarded-for", "not-an-ip")])),
            None
        );
    }
}
