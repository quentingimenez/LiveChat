//! Fuzz target `html_parsers`: feeds arbitrary input to the third-party HTML/JSON parsers.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    livechat_fuzz::html_parsers(data);
});
