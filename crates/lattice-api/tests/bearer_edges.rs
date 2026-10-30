//! Replays the `bearer_verify` fuzz seeds through the same checks the fuzz target makes, so a seed
//! that breaks an invariant fails in the pull request gate and not only in a scheduled fuzz run.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::disallowed_methods // the test reads the seed files from the repository
)]

use lattice_api::bearer::{CredentialStore, STORE_HEADER, verify_bearer};

const TOKEN: &[u8] = b"lat_0001020304050607_oKGio6SlpqeoqaqrrK2ur7CxsrO0tba3uLm6u7y9vr8";
const LINE: &str = "0001020304050607\talice\tactive\t1000\t-\t101112131415161718191a1b1c1d1e1f\t86e09220183af88b9f685b7a67f89a8099a88993781235e02fd4e2f6fa9dc589";

#[test]
fn every_seed_keeps_the_fuzz_target_invariants() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/bearer_verify");
    let fixed = CredentialStore::parse(&format!("{STORE_HEADER}\n{LINE}\n")).unwrap();
    let mut seen = 0;
    let mut accepted = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let data = std::fs::read(entry.unwrap().path()).unwrap();
        seen += 1;
        let (header, file) = match data.iter().position(|b| *b == 0) {
            Some(at) => (&data[..at], Some(&data[at + 1..])),
            None => (&data[..], None),
        };
        for now in [0, 1000, u64::MAX] {
            if let Ok(principal) = verify_bearer(Some(header), &fixed, now) {
                accepted += 1;
                assert_eq!(header[..7].to_ascii_lowercase(), b"bearer ");
                assert_eq!(&header[7..], TOKEN);
                assert_eq!(principal.name(), "alice");
            }
        }
        if let Some(Ok(text)) = file.map(std::str::from_utf8)
            && let Ok(store) = CredentialStore::parse(text)
        {
            assert_eq!(store.render(), text);
            let _ = verify_bearer(Some(header), &store, 1000);
        }
    }
    assert!(seen >= 20, "{seen} seeds");
    // The valid-header seeds are accepted (two times 1000 and 0 each), so the check above ran.
    assert!(accepted > 0);
}
