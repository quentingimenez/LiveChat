//! Fuzz entry points of the `livechat-fuzz` crate and their deterministic
//! seed test. Each `pub fn` below is driven by the matching target in
//! `fuzz/fuzz_targets/` and asserts the invariant the fuzzer must preserve.

use lc_protocol::{ClientMessage, Envelope, ServerMessage};

/// Envelope round-trip contract shared by both protocol message enums.
///
/// `serde` is not a direct dependency of this crate: the helper stays within
/// the dependencies already declared in `fuzz/Cargo.toml` (`libfuzzer-sys`,
/// `lc-protocol`, `lc-media`, `serde_json`), so `serde::Serialize` /
/// `serde::de::DeserializeOwned` cannot be named in a where clause. Passing
/// the three `serde_json` entry points as function pointers carries their
/// bounds from the concrete call sites while keeping a single generic body
/// for both message types.
fn envelope_roundtrip<M>(
    text: &str,
    parse: fn(&str) -> Result<Envelope<M>, serde_json::Error>,
    to_json: fn(&Envelope<M>) -> Result<serde_json::Value, serde_json::Error>,
    from_json: fn(serde_json::Value) -> Result<Envelope<M>, serde_json::Error>,
) where
    M: 'static,
{
    let Ok(envelope) = parse(text) else {
        return;
    };
    let Ok(v1) = to_json(&envelope) else {
        return;
    };
    let back: Envelope<M> = from_json(v1.clone()).unwrap_or_else(|err| {
        panic!(
            "Envelope<{}> did not survive the Value round-trip: {err}; input: {text:?}",
            std::any::type_name::<M>()
        )
    });
    let roundtripped = to_json(&back).unwrap_or_else(|err| {
        panic!(
            "Envelope<{}> failed to serialize after the Value round-trip: {err}; input: {text:?}",
            std::any::type_name::<M>()
        )
    });
    assert_eq!(
        roundtripped,
        v1,
        "Envelope<{}> is not stable across a Value round-trip; input: {text:?}",
        std::any::type_name::<M>()
    );
}

/// Round-trips any UTF-8 input that parses as an `Envelope<ClientMessage>`
/// or `Envelope<ServerMessage>` through `serde_json::Value` and asserts the
/// wire representation is stable.
pub fn protocol_frame(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    envelope_roundtrip::<ClientMessage>(
        text,
        |input| serde_json::from_str::<Envelope<ClientMessage>>(input),
        |envelope| serde_json::to_value(envelope),
        |value| serde_json::from_value::<Envelope<ClientMessage>>(value),
    );
    envelope_roundtrip::<ServerMessage>(
        text,
        |input| serde_json::from_str::<Envelope<ServerMessage>>(input),
        |envelope| serde_json::to_value(envelope),
        |value| serde_json::from_value::<Envelope<ServerMessage>>(value),
    );
}

/// Feeds UTF-8 input through the SSRF guard and asserts that every accepted
/// redirect result stays on `http`/`https`.
pub fn media_url(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(url) = lc_media::guard::parse_media_url(text) else {
        return;
    };
    let Ok(next) = lc_media::guard::resolve_redirect(&url, text) else {
        return;
    };
    assert!(
        matches!(next.scheme(), "http" | "https"),
        "resolved scheme `{}` is neither http nor https; input: {text:?}",
        next.scheme()
    );
}

/// Parses untrusted third-party HTML/JSON with byte indices; a panic on a
/// char boundary is the bug class this target looks for, so every result is
/// deliberately ignored.
pub fn html_parsers(data: &[u8]) {
    let s = String::from_utf8_lossy(data);
    let _ = lc_media::resolve::tiktok::parse_page_html(&s, "https://www.tiktok.com/@a/video/1");
    let _ = lc_media::resolve::tiktok::photo_slides_from_raw_html(&s);
    let _ = lc_media::resolve::tiktok::extract_item_id(&s);
    let _ = lc_media::resolve::twitter::sanitize_card_html(&s);
    let _ = lc_media::html::attributes(&s);
    let _ = lc_media::html::meta_content(&s, "og:video");
    let _ = lc_media::html::script_json(&s, "__NEXT_DATA__");
    let n = data.first().copied().unwrap_or(0) as usize;
    let _ = lc_media::html::find_ci(&s, "<script", n);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
        let _ = lc_media::resolve::twitter::tweet_card_content(&v);
        let _ = lc_media::resolve::twitter::collect_variants(&v);
        let _ = lc_media::resolve::twitter::collect_photos(&v);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use lc_protocol::{ClientMessage, Envelope, ServerMessage};

    use super::{html_parsers, media_url, protocol_frame};

    fn read_seed_dir(dir: &str) -> Vec<std::path::PathBuf> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|err| panic!("failed to read seed dir {dir}: {err}"))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        entries.into_iter().map(|entry| entry.path()).collect()
    }

    #[test]
    fn seeds_do_not_panic_or_violate_contracts() {
        let protocol_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/lc-protocol/tests/fixtures"
        );
        for path in read_seed_dir(protocol_dir) {
            let bytes = std::fs::read(&path).unwrap();
            protocol_frame(&bytes);
        }

        let media_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/lc-media/tests/fixtures"
        );
        for path in read_seed_dir(media_dir) {
            let bytes = std::fs::read(&path).unwrap();
            html_parsers(&bytes);
        }

        for url in [
            "https://www.tiktok.com/@a/video/1",
            "http://0x7f.1/",
            "http://[::ffff:127.0.0.1]/",
            "https://youtu.be/abc",
        ] {
            media_url(url.as_bytes());
        }
    }

    /// Every protocol fixture must reserialize to exactly its committed
    /// golden value: the on-wire shape is pinned by the fixture files, not
    /// by the product code alone. A coherent product-side serde drift (a
    /// renamed field, a changed serialization attribute) that keeps the
    /// `Envelope<M>` round-trip STABILITY intact — everything the old
    /// `seeds_do_not_panic_or_violate_contracts` invariant can see — is
    /// caught here, because the reserialized envelope no longer equals the
    /// golden file value.
    #[test]
    fn golden_protocol_fixtures_match_their_file_value() {
        let protocol_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/lc-protocol/tests/fixtures"
        );
        for path in read_seed_dir(protocol_dir) {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()));
            let file_value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|err| {
                panic!("fixture {} is not valid JSON: {err}", path.display())
            });
            let reserialized = if let Ok(envelope) =
                serde_json::from_str::<Envelope<ClientMessage>>(&text)
            {
                serde_json::to_value(&envelope).unwrap_or_else(|err| {
                    panic!("fixture {} failed to reserialize: {err}", path.display())
                })
            } else if let Ok(envelope) = serde_json::from_str::<Envelope<ServerMessage>>(&text) {
                serde_json::to_value(&envelope).unwrap_or_else(|err| {
                    panic!("fixture {} failed to reserialize: {err}", path.display())
                })
            } else {
                panic!(
                        "fixture {} parses as neither Envelope<ClientMessage> nor Envelope<ServerMessage>",
                        path.display()
                    );
            };
            assert_eq!(
                reserialized,
                file_value,
                "fixture {} drifted from its golden file value: reserialized {} vs file {}",
                path.display(),
                reserialized,
                file_value
            );
        }
    }
}
