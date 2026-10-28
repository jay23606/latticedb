#![no_main]
use lattice_core::changeset::{ChangeSet, decode_turn_begin};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(cs) = ChangeSet::decode(data) {
        // Whatever decodes must re-encode to something that decodes to the same value.
        if let Ok(bytes) = cs.encode() {
            assert_eq!(ChangeSet::decode(&bytes), Ok(cs));
        }
    }
    let _ = decode_turn_begin(data);
});
