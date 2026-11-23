//! Client IP announced by a trusted reverse proxy.
//!
//! Reading the headers requires two explicit settings: the feature flag
//! (`TRUST_PROXY_HEADERS` for the rate limit, `ADMIN_TRUST_FORWARDED_FOR` for
//! the admin local-only check) **and** the direct peer listed in
//! `TRUSTED_PROXIES`. [`proxied_client_ip`] only parses the headers; callers
//! must gate it with [`is_trusted_proxy`] first. A direct client can send both
//! headers itself, so reading them from an undeclared peer lets it choose its
//! rate-limit key (or claim loopback for the admin panel).
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
//!
//! The proxy itself must overwrite (not append) `X-Real-IP`: an honest proxy
//! that forwards the client header unchanged makes every value forgeable.

use std::net::IpAddr;

use axum::http::HeaderMap;

/// Client address from the proxy headers, or `None` when absent or invalid.
///
/// Pure parser: it does not know the peer. Only call it after
/// [`is_trusted_proxy`] accepted the direct peer.
pub fn proxied_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    let real_ip = headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<IpAddr>().ok());
    real_ip
        .or_else(|| {
            headers
                .get("x-forwarded-for")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.rsplit(',').next())
                .and_then(|hop| hop.trim().parse::<IpAddr>().ok())
        })
        .map(canonical_ip)
}

/// Normalizes an IPv4 address encapsulated in IPv6 (`::ffff:a.b.c.d`).
///
/// A dual-stack listening socket presents IPv4 peers in that form. Without
/// normalization those peers would not match their IPv4 entry in
/// `TRUSTED_PROXIES`, would not be recognized as loopback in development, and
/// would be counted separately from their real IPv4 address.
pub fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        IpAddr::V4(v4) => IpAddr::V4(v4),
    }
}

/// `true` when the direct peer is one of the declared reverse proxies.
///
/// The comparison is canonical, so a dual-stack peer `::ffff:127.0.0.1`
/// matches a declared `127.0.0.1`. An empty list trusts nobody, and a missing
/// peer (no `ConnectInfo`) is never trusted.
pub fn is_trusted_proxy(peer_ip: Option<IpAddr>, trusted_proxies: &[IpAddr]) -> bool {
    let Some(peer_ip) = peer_ip else {
        return false;
    };
    let peer_ip = canonical_ip(peer_ip);
    trusted_proxies
        .iter()
        .any(|trusted| canonical_ip(*trusted) == peer_ip)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

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

    #[test]
    fn canonical_ip_normalizes_ipv4_mapped_addresses() {
        let mapped: IpAddr = "::ffff:203.0.113.7".parse().expect("adresse valide");
        assert_eq!(
            canonical_ip(mapped),
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))
        );

        let ipv4: IpAddr = "192.0.2.10".parse().expect("adresse valide");
        assert_eq!(canonical_ip(ipv4), ipv4);

        let ipv6: IpAddr = "2001:db8::1".parse().expect("adresse valide");
        assert_eq!(canonical_ip(ipv6), ipv6);
    }

    #[test]
    fn only_declared_peers_are_trusted() {
        let trusted: Vec<IpAddr> = ["127.0.0.1", "::1", "2001:db8::1"]
            .iter()
            .map(|ip| ip.parse().expect("adresse valide"))
            .collect();

        assert!(is_trusted_proxy(
            Some("127.0.0.1".parse().unwrap()),
            &trusted
        ));
        assert!(is_trusted_proxy(Some("::1".parse().unwrap()), &trusted));
        // A dual-stack socket presents an IPv4 peer in mapped form.
        assert!(is_trusted_proxy(
            Some("::ffff:127.0.0.1".parse().unwrap()),
            &trusted
        ));
        assert!(!is_trusted_proxy(
            Some("203.0.113.9".parse().unwrap()),
            &trusted
        ));
        assert!(!is_trusted_proxy(None, &trusted));
        assert!(!is_trusted_proxy(Some("127.0.0.1".parse().unwrap()), &[]));
    }

    #[test]
    fn forwarded_addresses_are_canonicalized() {
        let map = headers(&[("x-real-ip", "::ffff:203.0.113.7")]);
        assert_eq!(
            proxied_client_ip(&map),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)))
        );
    }
}
