#![no_main]
use lattice_core::changeset::ChangeSetId;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data)
        && let Ok(id) = ChangeSetId::parse(s)
    {
        assert_eq!(ChangeSetId::parse(&id.to_string()), Ok(id));
    }
});
