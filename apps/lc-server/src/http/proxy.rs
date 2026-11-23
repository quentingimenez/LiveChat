//! Client IP announced by a trusted reverse proxy.
//!
//! Reading the headers requires two explicit settings: the feature flag
//! (`TRUST_PROXY_HEADERS` for the rate limit, `ADMIN_TRUST_FORWARDED_FOR` for
//! the admin local-only check) **and** the direct peer listed in
//! `TRUSTED_PROXIES`. [`forwarded_client_ip`] enforces both the peer check and
//! the chain walk; callers still decide whether their flag enables it. A direct
//! client can send the headers itself, so reading them from an undeclared peer
//! would let it choose its rate-limit key (or claim loopback for the admin
//! panel).
//!
//! `X-Forwarded-For` is a chain, oldest hop first: the client writes the first
//! entries, then every proxy **appends** the address of its direct peer. Reading
//! only the last entry is not enough when two declared proxies are chained: the
//! last entry is then the outer proxy, and every client behind it shares one
//! rate-limit bucket. [`forwarded_client_ip`] therefore walks the chain from
//! right to left, continues while the current hop is itself a declared proxy,
//! and returns the first hop that is not one. An invalid or non-UTF-8 entry
//! stops the walk (fail closed): nothing to its left can be attributed to a
//! declared proxy, and an attacker could otherwise hide a forged hop behind a
//! malformed value.
//!
//! `X-Real-IP` is only a single-proxy fallback, used when `X-Forwarded-For` has
//! no usable entry. When both are present the chain wins; otherwise the nearest
//! proxy could hide the real client (or an outer proxy) behind a header that
//! does not describe the whole chain.
//!
//! The proxy must overwrite (not forward unchanged) `X-Real-IP` and append to
//! `X-Forwarded-For`: an honest proxy that forwards the client values unchanged
//! makes every value forgeable.

use std::net::IpAddr;

use axum::http::HeaderMap;

/// Client address resolved from the proxy chain, or `None`.
///
/// Returns `None` when the direct peer is not declared in `trusted_proxies`,
/// when no non-proxy hop exists, or when an entry is invalid (fail closed).
/// `X-Real-IP` is only read when `X-Forwarded-For` carries no hop at all.
/// Returned addresses are canonicalized.
pub fn forwarded_client_ip(
    peer_ip: Option<IpAddr>,
    headers: &HeaderMap,
    trusted_proxies: &[IpAddr],
) -> Option<IpAddr> {
    if !is_trusted_proxy(peer_ip, trusted_proxies) {
        return None;
    }

    let mut hops = Vec::new();
    for value in headers.get_all("x-forwarded-for") {
        let Ok(value) = value.to_str() else {
            return None;
        };
        for hop in value.split(',') {
            let hop = hop.trim();
            if hop.is_empty() {
                continue;
            }
            match hop.parse::<IpAddr>() {
                Ok(ip) => hops.push(canonical_ip(ip)),
                Err(_) => return None,
            }
        }
    }

    if hops.is_empty() {
        return headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
            .map(canonical_ip);
    }

    for hop in hops.iter().rev() {
        if !is_trusted_proxy(Some(*hop), trusted_proxies) {
            return Some(*hop);
        }
    }
    None
}

/// Address to journal for a request.
///
/// An undeclared direct peer is the client itself. A declared proxy is never
/// labelled as the client: only a hop resolved by [`forwarded_client_ip`] is
/// journaled, so a chain that cannot be resolved gives `None`.
pub fn audit_client_ip(
    peer_ip: Option<IpAddr>,
    headers: &HeaderMap,
    trusted_proxies: &[IpAddr],
) -> Option<IpAddr> {
    let peer_ip = peer_ip.map(canonical_ip);
    if is_trusted_proxy(peer_ip, trusted_proxies) {
        forwarded_client_ip(peer_ip, headers, trusted_proxies)
    } else {
        peer_ip
    }
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

    use std::net::{Ipv4Addr, Ipv6Addr};

    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, HeaderValue::from_static(value));
        }
        map
    }

    fn trusted(addresses: &[&str]) -> Vec<IpAddr> {
        addresses
            .iter()
            .map(|ip| ip.parse().expect("adresse valide"))
            .collect()
    }

    fn peer(address: &str) -> Option<IpAddr> {
        Some(address.parse().expect("adresse valide"))
    }

    fn v4(address: &str) -> Option<IpAddr> {
        Some(IpAddr::V4(
            address.parse::<Ipv4Addr>().expect("adresse valide"),
        ))
    }

    #[test]
    fn two_declared_proxies_resolve_the_original_client() {
        // server <- 10.0.0.2 <- 10.0.0.9 <- 198.51.100.4, with a forged
        // 127.0.0.1 as the leftmost entry.
        let map = headers(&[("x-forwarded-for", "127.0.0.1, 198.51.100.4, 10.0.0.9")]);
        assert_eq!(
            forwarded_client_ip(peer("10.0.0.2"), &map, &trusted(&["10.0.0.2", "10.0.0.9"])),
            v4("198.51.100.4")
        );
    }

    #[test]
    fn a_forged_left_hop_is_never_trusted() {
        // Single declared proxy; the client tries to claim loopback.
        let map = headers(&[("x-forwarded-for", "127.0.0.1, 203.0.113.7")]);
        assert_eq!(
            forwarded_client_ip(peer("127.0.0.1"), &map, &trusted(&["127.0.0.1"])),
            v4("203.0.113.7")
        );
    }

    #[test]
    fn an_undeclared_peer_never_contributes_headers() {
        let map = headers(&[
            ("x-forwarded-for", "127.0.0.1, 203.0.113.7"),
            ("x-real-ip", "127.0.0.1"),
        ]);
        assert_eq!(
            forwarded_client_ip(peer("203.0.113.9"), &map, &trusted(&["127.0.0.1"])),
            None
        );
        // Even a chain that is entirely declared is not read for this peer.
        assert_eq!(
            forwarded_client_ip(
                peer("203.0.113.9"),
                &headers(&[("x-forwarded-for", "127.0.0.1")]),
                &trusted(&["127.0.0.1"])
            ),
            None
        );
    }

    #[test]
    fn a_fully_declared_chain_has_no_client() {
        // Every entry is a proxy address: there is no client to return.
        let map = headers(&[
            ("x-forwarded-for", "10.0.0.9"),
            ("x-real-ip", "198.51.100.4"),
        ]);
        assert_eq!(
            forwarded_client_ip(peer("10.0.0.2"), &map, &trusted(&["10.0.0.2", "10.0.0.9"])),
            None
        );
    }

    #[test]
    fn an_invalid_hop_fails_closed() {
        // The rightmost entry is not an IP: the chain cannot be walked.
        assert_eq!(
            forwarded_client_ip(
                peer("127.0.0.1"),
                &headers(&[("x-forwarded-for", "203.0.113.7, not-an-ip")]),
                &trusted(&["127.0.0.1"])
            ),
            None
        );
        // Non-UTF-8 bytes are ignored by `to_str`; treat them as invalid.
        let mut map = HeaderMap::new();
        map.append(
            "x-forwarded-for",
            HeaderValue::from_bytes(b"\xff").expect("valeur opaque acceptée"),
        );
        assert_eq!(
            forwarded_client_ip(peer("127.0.0.1"), &map, &trusted(&["127.0.0.1"])),
            None
        );
    }

    #[test]
    fn x_real_ip_is_only_a_single_proxy_fallback() {
        // No forwarded chain: the proxy's own header is the only source.
        let map = headers(&[("x-real-ip", "198.51.100.4")]);
        assert_eq!(
            forwarded_client_ip(peer("127.0.0.1"), &map, &trusted(&["127.0.0.1"])),
            v4("198.51.100.4")
        );

        // A real chain wins over X-Real-IP, which may describe an inner proxy.
        let map = headers(&[
            ("x-forwarded-for", "198.51.100.4, 10.0.0.9"),
            ("x-real-ip", "10.0.0.9"),
        ]);
        assert_eq!(
            forwarded_client_ip(peer("10.0.0.2"), &map, &trusted(&["10.0.0.2", "10.0.0.9"])),
            v4("198.51.100.4")
        );
    }

    #[test]
    fn multiple_forwarded_for_headers_are_concatenated() {
        let map = headers(&[
            ("x-forwarded-for", "198.51.100.4"),
            ("x-forwarded-for", "10.0.0.9"),
        ]);
        assert_eq!(
            forwarded_client_ip(peer("10.0.0.2"), &map, &trusted(&["10.0.0.2", "10.0.0.9"])),
            v4("198.51.100.4")
        );
    }

    #[test]
    fn missing_or_garbage_headers_give_none() {
        assert_eq!(
            forwarded_client_ip(
                peer("127.0.0.1"),
                &HeaderMap::new(),
                &trusted(&["127.0.0.1"])
            ),
            None
        );
        assert_eq!(
            forwarded_client_ip(
                peer("127.0.0.1"),
                &headers(&[("x-forwarded-for", "not-an-ip")]),
                &trusted(&["127.0.0.1"])
            ),
            None
        );
        assert_eq!(
            forwarded_client_ip(
                peer("127.0.0.1"),
                &headers(&[("x-real-ip", "unknown")]),
                &trusted(&["127.0.0.1"])
            ),
            None
        );
    }

    #[test]
    fn forwarded_addresses_are_canonicalized() {
        let map = headers(&[("x-real-ip", "::ffff:203.0.113.7")]);
        assert_eq!(
            forwarded_client_ip(peer("::ffff:127.0.0.1"), &map, &trusted(&["127.0.0.1"])),
            v4("203.0.113.7")
        );

        let map = headers(&[("x-forwarded-for", "::ffff:203.0.113.7")]);
        assert_eq!(
            forwarded_client_ip(peer("127.0.0.1"), &map, &trusted(&["127.0.0.1"])),
            v4("203.0.113.7")
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

        let ipv6: IpAddr = Ipv6Addr::LOCALHOST.into();
        assert_eq!(canonical_ip(ipv6), ipv6);
    }

    #[test]
    fn only_declared_peers_are_trusted() {
        let trusted = trusted(&["127.0.0.1", "::1", "2001:db8::1"]);

        assert!(is_trusted_proxy(peer("127.0.0.1"), &trusted));
        assert!(is_trusted_proxy(peer("::1"), &trusted));
        // A dual-stack socket presents an IPv4 peer in mapped form.
        assert!(is_trusted_proxy(peer("::ffff:127.0.0.1"), &trusted));
        assert!(!is_trusted_proxy(peer("203.0.113.9"), &trusted));
        assert!(!is_trusted_proxy(None, &trusted));
        assert!(!is_trusted_proxy(peer("127.0.0.1"), &[]));
    }

    #[test]
    fn audit_never_labels_a_declared_proxy_as_the_client() {
        let trusted = trusted(&["127.0.0.1", "10.0.0.9"]);

        // An undeclared peer is its own client, headers are irrelevant.
        assert_eq!(
            audit_client_ip(
                peer("203.0.113.9"),
                &headers(&[("x-forwarded-for", "127.0.0.1")]),
                &trusted
            ),
            v4("203.0.113.9")
        );

        // A declared proxy is replaced by the client at the end of the chain.
        assert_eq!(
            audit_client_ip(
                peer("127.0.0.1"),
                &headers(&[("x-forwarded-for", "198.51.100.7, 10.0.0.9")]),
                &trusted
            ),
            v4("198.51.100.7")
        );

        // No resolvable client: journal nothing rather than the proxy address.
        assert_eq!(
            audit_client_ip(peer("127.0.0.1"), &HeaderMap::new(), &trusted),
            None
        );
        assert_eq!(
            audit_client_ip(
                peer("127.0.0.1"),
                &headers(&[("x-forwarded-for", "10.0.0.9")]),
                &trusted
            ),
            None
        );
    }
}
