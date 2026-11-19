//! Fuzz target `media_url`: exercises the SSRF guard on arbitrary input.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    livechat_fuzz::media_url(data);
});
