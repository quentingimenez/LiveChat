//! Property-based tests for the `lc-media` SSRF guard and `yt-dlp` argument
//! building.
//!
//! Every property reads the security contract of a pure helper and checks it
//! over a whole generated input space, complementing the example-based unit
//! tests inside `src/`:
//!
//! - any address generated inside each blocked IPv4/IPv6 range is rejected;
//! - an IPv4-mapped IPv6 address always carries the verdict of its IPv4;
//! - every alternate IPv4 host spelling (single `u32`, `0x` hex, dotted
//!   octal, dotted hex) parses to the blocked literal it encodes;
//! - `parse_media_url` never panics and only accepts bare `http`/`https`;
//! - `resolve_redirect` never produces a non-HTTP(S) target;
//! - `build_ytdlp_args` always places the url last, right after a `--`
//!   terminator, so the url can never be read as a `yt-dlp` option.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use lc_media::config::MediaConfig;
use lc_media::download::{build_ytdlp_args, YtdlpOptions};
use lc_media::error::MediaErrorCode;
use lc_media::guard::{
    ip_block_reason, parse_media_url, resolve_redirect, ValidatedTarget, MAX_ERROR_BODY_BYTES,
    MAX_TEXT_RESPONSE_BYTES,
};
use proptest::prelude::*;
// Extra imports besides the prelude: the regex-backed string strategy and
// the error type used to fail a single generated case without aborting the
// whole test binary.
use proptest::string::string_regex;
use proptest::test_runner::TestCaseError;
use url::{Host, Url};

/// Number of distinct blocked IPv4 ranges covered by [`blocked_ipv4`].
const BLOCKED_V4_RANGE_COUNT: usize = 13;
/// Number of distinct blocked IPv6 kinds covered by [`blocked_ipv6`].
const BLOCKED_V6_KIND_COUNT: usize = 6;

/// Builds an address inside blocked IPv4 range number `range`.
///
/// Each branch pins exactly the octets its CIDR range defines and lets the
/// remaining filler bytes vary, so the generator can produce an address in
/// every range listed by the SSRF policy.
fn blocked_ipv4(range: usize, fill: [u8; 4]) -> Ipv4Addr {
    let [a, b, c, d] = fill;
    match range {
        0 => Ipv4Addr::new(0, b, c, d),                 // 0.0.0.0/8
        1 => Ipv4Addr::new(10, b, c, d),                // 10.0.0.0/8
        2 => Ipv4Addr::new(100, 64 | (a & 0x3f), c, d), // 100.64.0.0/10
        3 => Ipv4Addr::new(127, b, c, d),               // 127.0.0.0/8
        4 => Ipv4Addr::new(169, 254, c, d),             // 169.254.0.0/16
        5 => Ipv4Addr::new(172, 16 | (a & 0x0f), c, d), // 172.16.0.0/12
        6 => Ipv4Addr::new(192, 0, 2, d),               // 192.0.2.0/24
        7 => Ipv4Addr::new(192, 168, c, d),             // 192.168.0.0/16
        8 => Ipv4Addr::new(198, 18 | (a & 0x01), c, d), // 198.18.0.0/15
        9 => Ipv4Addr::new(198, 51, 100, d),            // 198.51.100.0/24
        10 => Ipv4Addr::new(203, 0, 113, d),            // 203.0.113.0/24
        11 => Ipv4Addr::new(224 | (a & 0x0f), b, c, d), // 224.0.0.0/4
        12 => Ipv4Addr::new(240 | (a & 0x0f), b, c, d), // 240.0.0.0/4
        other => panic!("unknown blocked IPv4 range index: {other}"),
    }
}

/// Builds an address inside blocked IPv6 kind number `kind`.
///
/// Fixed kinds (`::1`, `::`) plus generated addresses inside every listed
/// prefix; the low bits vary so the whole range is covered.
fn blocked_ipv6(kind: usize, a: u16, b: u16, c: u16) -> Ipv6Addr {
    match kind {
        0 => Ipv6Addr::LOCALHOST,                                       // ::1
        1 => Ipv6Addr::UNSPECIFIED,                                     // ::
        2 => Ipv6Addr::new(0xfc00 | (a & 0x01ff), b, c, 0, 0, 0, 0, 0), // fc00::/7
        3 => Ipv6Addr::new(0xfe80 | (a & 0x003f), b, c, 0, 0, 0, 0, 0), // fe80::/10
        4 => Ipv6Addr::new(0xff00 | (a & 0x00ff), b, c, 0, 0, 0, 0, 0), // ff00::/8
        5 => Ipv6Addr::new(0x2001, 0x0db8, a, b, c, 0, 0, 0),           // 2001:db8::/32
        other => panic!("unknown blocked IPv6 kind index: {other}"),
    }
}

/// Generates one address in each blocked IPv4 range across the case space.
fn blocked_ipv4_strategy() -> impl Strategy<Value = Ipv4Addr> {
    (
        0..BLOCKED_V4_RANGE_COUNT,
        any::<u8>(),
        any::<u8>(),
        any::<u8>(),
        any::<u8>(),
    )
        .prop_map(|(range, a, b, c, d)| blocked_ipv4(range, [a, b, c, d]))
}

/// Generates one address in each blocked IPv6 range across the case space.
fn blocked_ipv6_strategy() -> impl Strategy<Value = Ipv6Addr> {
    (
        0..BLOCKED_V6_KIND_COUNT,
        any::<u16>(),
        any::<u16>(),
        any::<u16>(),
    )
        .prop_map(|(kind, a, b, c)| blocked_ipv6(kind, a, b, c))
}

/// Every alternate host spelling of `ip` accepted by the URL specification.
///
/// Browsers normalize all of these forms to the same IPv4 literal, so a
/// client may hand any of them to the pipeline and the guard must still see
/// the blocked address the spelling encodes.
fn blocked_ipv4_encodings(ip: Ipv4Addr) -> Vec<String> {
    let [a, b, c, d] = ip.octets();
    let numeric = u32::from(ip);
    vec![
        format!("{ip}"),                            // dotted decimal
        numeric.to_string(),                        // single decimal u32
        format!("0x{numeric:x}"),                   // single 0x hex u32
        format!("0{a:o}.0{b:o}.0{c:o}.0{d:o}"),     // dotted octal, leading 0 per octet
        format!("0x{a:x}.0x{b:x}.0x{c:x}.0x{d:x}"), // dotted 0x hex
    ]
}

/// Arbitrary strings: fully random values, plus any code point outside the
/// Unicode `C` category (the `\PC` regex class) up to 200 characters.
fn arbitrary_strings() -> BoxedStrategy<String> {
    let any_codepoint = string_regex("\\PC{0,200}").expect("proptest supports the \\PC class");
    prop_oneof![
        2 => any::<String>(),
        1 => any_codepoint,
    ]
    .boxed()
}

/// Urls handed to `yt-dlp`: arbitrary strings plus explicitly option-shaped
/// ones (starting with `-` or `--`) that must end up after the `--`
/// terminator, never parsed as a `yt-dlp` option.
fn ytdlp_urls() -> BoxedStrategy<String> {
    let any_codepoint = string_regex("\\PC{0,80}").expect("proptest supports the \\PC class");
    prop_oneof![
        4 => any::<String>(),
        2 => any_codepoint,
        1 => Just("https://youtu.be/dQw4w9WgXcQ".to_string()),
        1 => Just("-oC:\\evil\\path.mp4".to_string()),
        1 => Just("--print".to_string()),
        1 => Just("--".to_string()),
    ]
    .boxed()
}

proptest! {
    /// Any address inside a blocked IPv4 range must be rejected.
    #[test]
    fn blocked_ipv4_ranges_are_always_rejected(ip in blocked_ipv4_strategy()) {
        prop_assert!(
            ip_block_reason(IpAddr::V4(ip)).is_some(),
            "blocked IPv4 {ip} was accepted"
        );
    }

    /// Any address inside a blocked IPv6 range must be rejected.
    #[test]
    fn blocked_ipv6_ranges_are_always_rejected(ip in blocked_ipv6_strategy()) {
        prop_assert!(
            ip_block_reason(IpAddr::V6(ip)).is_some(),
            "blocked IPv6 {ip} was accepted"
        );
    }

    /// An IPv4-mapped IPv6 address always carries the verdict of its IPv4.
    #[test]
    fn ipv4_mapped_ipv6_has_the_ipv4_verdict(raw in any::<u32>()) {
        let ipv4 = Ipv4Addr::from(raw);
        let direct = ip_block_reason(IpAddr::V4(ipv4)).is_some();
        let mapped = ip_block_reason(IpAddr::V6(ipv4.to_ipv6_mapped())).is_some();
        prop_assert_eq!(
            direct,
            mapped,
            "IPv4-mapped {} disagrees with direct {}",
            ipv4.to_ipv6_mapped(),
            ipv4
        );
    }

    /// Every alternate IPv4 host spelling parses to the blocked literal it
    /// encodes, and that literal is rejected.
    #[test]
    fn encoded_blocked_ipv4_hosts_parse_to_the_blocked_ip(ip in blocked_ipv4_strategy()) {
        for encoding in blocked_ipv4_encodings(ip) {
            let raw = format!("http://{encoding}/x");
            match parse_media_url(&raw) {
                Ok(url) => match url.host() {
                    Some(Host::Ipv4(parsed)) => {
                        prop_assert_eq!(parsed, ip, "encoding {} parsed to {}", raw, parsed);
                        prop_assert!(
                            ip_block_reason(IpAddr::V4(parsed)).is_some(),
                            "parsed host {parsed} of {raw} is not blocked"
                        );
                    }
                    other => {
                        return Err(TestCaseError::fail(format!(
                            "host of {raw} parsed to {other:?}, expected Ipv4"
                        )));
                    }
                },
                Err(error) => {
                    return Err(TestCaseError::fail(format!(
                        "blocked encoding {raw} rejected: {error}"
                    )));
                }
            }
        }
    }

    /// `parse_media_url` never panics, and whatever it accepts is a bare
    /// `http`/`https` URL with a host and no embedded credentials.
    #[test]
    fn parse_media_url_never_panics_and_only_accepts_http(raw in arbitrary_strings()) {
        if let Ok(url) = parse_media_url(&raw) {
            prop_assert!(
                matches!(url.scheme(), "http" | "https"),
                "accepted a non-HTTP(S) URL: {raw}"
            );
            prop_assert!(url.host().is_some(), "accepted a hostless URL: {raw}");
            prop_assert!(url.username().is_empty(), "accepted a username in: {raw}");
            prop_assert!(url.password().is_none(), "accepted a password in: {raw}");
        }
    }

    /// A resolved redirect never escapes `http`/`https`.
    #[test]
    fn redirects_stay_http_or_https(location in arbitrary_strings()) {
        let current = Url::parse("https://example.com/a/b").expect("valid base URL");
        if let Ok(next) = resolve_redirect(&current, &location) {
            prop_assert!(
                matches!(next.scheme(), "http" | "https"),
                "redirect location {location:?} produced scheme {}",
                next.scheme()
            );
        }
    }

    /// The url handed to `yt-dlp` is always the last argument, immediately
    /// after a `--` terminator, so it can never be read as an option — even
    /// when it starts with `-`.
    #[test]
    fn ytdlp_url_comes_last_after_double_dash(url in ytdlp_urls()) {
        let config = MediaConfig::default();
        let temp_dir = Path::new("C:\\temp\\lc-media");
        let args = build_ytdlp_args(&config, &url, temp_dir, &YtdlpOptions::default());
        prop_assert_eq!(
            args.last(),
            Some(&url),
            "url {:?} is not the last yt-dlp argument: {:?}",
            url,
            args
        );
        let before_url = args.len().checked_sub(2).and_then(|index| args.get(index));
        prop_assert_eq!(
            before_url,
            Some(&"--".to_string()),
            "no `--` terminator right before the url: {:?}",
            args
        );
    }
}

// ---------------------------------------------------------------------------
// Acceptance properties: a generated PUBLIC address must be accepted
// ---------------------------------------------------------------------------

/// First octets that no blocked IPv4 family can ever start with.
///
/// The SSRF policy blocks, by first octet: `0.0.0.0/8` ("this network",
/// which also covers the unspecified address), `10.0.0.0/8` (RFC1918
/// private), `100.64.0.0/10` (CGNAT), `127.0.0.0/8` (loopback),
/// `169.254.0.0/16` (link-local / cloud metadata), `172.16.0.0/12` (RFC1918
/// private), `192.0.2.0/24`, `198.51.100.0/24` and `203.0.113.0/24`
/// (documentation), `192.168.0.0/16` (RFC1918 private), `198.18.0.0/15`
/// (benchmarking), `224.0.0.0/4` (multicast) and `240.0.0.0/4` (reserved,
/// broadcast included). Every range below starts inside `1..=223` while
/// skipping each blocked first octet — `10`, `100`, `127`, `169`, `172`,
/// `192`, `198`, `203` — and everything from `224` on, so an address built
/// from one of them lies outside every blocked family by construction and
/// its three filler octets can vary over the whole `0..=255` space.
fn public_ipv4_first_octet() -> BoxedStrategy<u8> {
    prop_oneof![
        1 => 1..=9u8,
        1 => 11..=99u8,
        1 => 101..=126u8,
        1 => 128..=168u8,
        1 => 170..=171u8,
        1 => 173..=191u8,
        1 => 193..=197u8,
        1 => 199..=202u8,
        1 => 204..=223u8,
    ]
    .boxed()
}

/// Public IPv4 addresses: a first octet outside every blocked family and
/// three fully arbitrary octets.
fn public_ipv4_strategy() -> impl Strategy<Value = Ipv4Addr> {
    (
        public_ipv4_first_octet(),
        any::<u8>(),
        any::<u8>(),
        any::<u8>(),
    )
        .prop_map(|(a, b, c, d)| Ipv4Addr::new(a, b, c, d))
}

proptest! {
    /// An address built from a first octet outside every blocked family is
    /// public: the guard must accept it.
    #[test]
    fn public_ipv4_addresses_are_accepted(ip in public_ipv4_strategy()) {
        prop_assert!(
            ip_block_reason(IpAddr::V4(ip)).is_none(),
            "public IPv4 {ip} was rejected"
        );
    }

    /// An address whose first byte falls in `0x21..=0xFB` is public: that
    /// range excludes every blocked IPv6 kind. Bytes `0x00..=0x20` exclude
    /// `::` (unspecified), `::1` (loopback), every IPv4-mapped address
    /// (their first two bytes are zero) and the documentation range
    /// `2001:db8::/32` (first byte `0x20`); byte `0xFC`/`0xFD` excludes
    /// `fc00::/7` (ULA); byte `0xFE` excludes `fe80::/10` (link-local);
    /// byte `0xFF` excludes `ff00::/8` (multicast). The fifteen remaining
    /// bytes vary freely.
    #[test]
    fn public_ipv6_addresses_are_accepted(
        first in 0x21u8..=0xFB,
        rest in proptest::collection::vec(any::<u8>(), 15),
    ) {
        let mut octets = [0_u8; 16];
        octets[0] = first;
        octets[1..].copy_from_slice(&rest);
        let ip = Ipv6Addr::from(octets);
        prop_assert!(
            ip_block_reason(IpAddr::V6(ip)).is_none(),
            "public IPv6 {ip} was rejected"
        );
    }

    /// `parse_media_url` accepts a public IPv4 host on both bare schemes.
    #[test]
    fn parse_media_url_accepts_public_hosts(ip in public_ipv4_strategy()) {
        for raw in [format!("https://{ip}/x"), format!("http://{ip}/x")] {
            let outcome = parse_media_url(&raw);
            prop_assert!(
                outcome.is_ok(),
                "public host URL {raw} was rejected: {:?}",
                outcome.err()
            );
        }
    }

    /// Embedded credentials make a source unreadable whatever the host: a
    /// non-empty username OR a present password is refused by the policy
    /// step of `parse_media_url` (`MediaErrorCode::UnsupportedSource`).
    #[test]
    fn urls_with_embedded_credentials_are_rejected(ip in public_ipv4_strategy()) {
        for raw in [
            format!("https://user@{ip}/"),
            format!("https://:pw@{ip}/"),
            format!("https://user:pw@{ip}/"),
        ] {
            match parse_media_url(&raw) {
                Ok(_) => {
                    return Err(TestCaseError::fail(format!(
                        "URL with embedded credentials was accepted: {raw}"
                    )));
                }
                Err(error) => {
                    prop_assert_eq!(
                        error.code(),
                        MediaErrorCode::UnsupportedSource,
                        "URL {} was rejected for the wrong reason: {}",
                        raw,
                        error
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Missed-mutant kills: policy pins and documentation-range neighbors
// ---------------------------------------------------------------------------

/// The response-size limits are guard policy, not incidental constants:
/// raising them changes what a redirect body may do to memory, so the
/// documented values (4 MiB of text, an 8 KiB error excerpt) are pinned.
#[test]
fn text_response_limits_pin_their_documented_values() {
    assert_eq!(MAX_TEXT_RESPONSE_BYTES, 4_194_304);
    assert_eq!(MAX_ERROR_BODY_BYTES, 8_192);
}

/// `url_str` is the text handed to the downloader and the logs: it must
/// be the canonical form of the validated URL, never a placeholder.
#[test]
fn url_str_returns_the_canonical_url_text() {
    let url = parse_media_url("https://example.com/x").unwrap();
    let target = ValidatedTarget {
        url,
        host: "example.com".to_string(),
        port: 443,
        addrs: Vec::new(),
    };
    assert_eq!(target.url_str(), "https://example.com/x");
}

/// Public addresses inside the /8s that also host blocked ranges: only
/// the documented sub-ranges of 192/8, 198/8 and 203/8 are refused
/// (`192.0.2.0/24`, `192.168.0.0/16`, `198.51.100.0/24`, `198.18.0.0/15`,
/// `203.0.113.0/24`), the rest of each /8 is public.
fn documentation_range_neighbor() -> impl Strategy<Value = Ipv4Addr> {
    prop_oneof![
        1 => (Just(192u8), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_filter("public 192/8 address", |&(_, second, third, _)| {
                !(second == 0 && third == 2) && second != 168
            }),
        1 => (Just(198u8), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_filter("public 198/8 address", |&(_, second, third, _)| {
                !(second == 51 && third == 100) && !(second == 18 || second == 19)
            }),
        1 => (Just(203u8), any::<u8>(), any::<u8>(), any::<u8>())
            .prop_filter("public 203/8 address", |&(_, second, third, _)| {
                !(second == 0 && third == 113)
            }),
    ]
    .prop_map(|(first, second, third, fourth)| Ipv4Addr::new(first, second, third, fourth))
}

proptest! {
    /// `192.100.x.x` stays public even though `192.0.2.0/24` and
    /// `192.168.0.0/16` are blocked, and the same holds for the neighbors
    /// of `198.51.100.0/24`, `198.18.0.0/15` and `203.0.113.0/24`. A block
    /// keyed on the first octet alone — a weakened `&&` turned into `||` —
    /// over-blocks a whole /8 and fails here.
    #[test]
    fn documentation_range_neighbors_are_public(ip in documentation_range_neighbor()) {
        prop_assert!(
            ip_block_reason(IpAddr::V4(ip)).is_none(),
            "public IPv4 {ip} neighboring a blocked range was rejected"
        );
    }
}

/// The IPv6 reason labels are user-facing policy text, and they keep a
/// plain loopback distinct from an IPv4-mapped one: deleting a mapped
/// arm of `ipv6_block_reason`, or weakening the mapped detection itself
/// (which re-routes `::1` through the IPv4 rules), changes the label and
/// fails this pin.
#[test]
fn blocked_ipv6_reasons_pin_their_documented_labels() {
    let loopback: Ipv6Addr = "::1".parse().unwrap();
    assert_eq!(ip_block_reason(IpAddr::V6(loopback)), Some("boucle locale"));

    let mapped_loopback: Ipv6Addr = "::ffff:127.0.0.1".parse().unwrap();
    assert_eq!(
        ip_block_reason(IpAddr::V6(mapped_loopback)),
        Some("boucle locale (IPv4 mappée)")
    );

    let mapped_private: Ipv6Addr = "::ffff:10.0.0.1".parse().unwrap();
    assert_eq!(
        ip_block_reason(IpAddr::V6(mapped_private)),
        Some("réseau privé IPv4 mappé (RFC1918)")
    );
}
