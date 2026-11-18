//! Fuzz target `protocol_frame`: placeholder written with the dependency
//! setup; Task 4 of `docs/superpowers/plans/2026-11-18-verification-pipeline.md`
//! replaces the body.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = data;
});
