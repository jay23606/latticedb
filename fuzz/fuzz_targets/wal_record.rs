#![no_main]
use lattice_core::record::decode_record;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = decode_record(data, 0);
    let _ = decode_record(data, data.len() / 2);
});
