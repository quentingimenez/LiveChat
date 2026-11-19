//! Fuzz target `protocol_frame`: round-trips protocol envelopes through serde_json.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    livechat_fuzz::protocol_frame(data);
});
