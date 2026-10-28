#![no_main]
use libfuzzer_sys::fuzz_target;

// Phase 0 placeholder target: proves the fuzz pipeline runs. Phase 1 adds the real decoders.
fuzz_target!(|data: &[u8]| {
    let _ = std::str::from_utf8(data);
});
