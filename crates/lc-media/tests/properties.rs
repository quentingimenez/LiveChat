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
use lc_media::guard::{ip_block_reason, parse_media_url, resolve_redirect};
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
