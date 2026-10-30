#![no_main]
use lattice_api::bearer::{CredentialStore, STORE_HEADER, verify_bearer};
use libfuzzer_sys::fuzz_target;

// Hand-made credential line and its token (see the unit tests in lattice-api::bearer).
const TOKEN: &[u8] = b"lat_0001020304050607_oKGio6SlpqeoqaqrrK2ur7CxsrO0tba3uLm6u7y9vr8";
const LINE: &str = "0001020304050607\talice\tactive\t1000\t-\t101112131415161718191a1b1c1d1e1f\t86e09220183af88b9f685b7a67f89a8099a88993781235e02fd4e2f6fa9dc589";

fuzz_target!(|data: &[u8]| {
    // The input is `header NUL file`. Without a NUL it is a header checked against a fixed store.
    let (header, file) = match data.iter().position(|b| *b == 0) {
        Some(at) => (&data[..at], Some(&data[at + 1..])),
        None => (data, None),
    };

    let fixed = CredentialStore::parse(&format!("{STORE_HEADER}\n{LINE}\n")).expect("fixed store parses");
    for now in [0, 1000, u64::MAX] {
        // Only the one stored token may authenticate, however the header is mangled.
        if let Ok(principal) = verify_bearer(Some(header), &fixed, now) {
            assert!(header.len() == 7 + TOKEN.len() && header[..7].eq_ignore_ascii_case(b"Bearer "));
            assert_eq!(&header[7..], TOKEN);
            assert_eq!(principal.name(), "alice");
        }
    }

    // A credential file: parse or refuse, never panic; a parsed store renders back to the same
    // bytes (one canonical text) and answers any header without panicking.
    if let Some(Ok(text)) = file.map(std::str::from_utf8)
        && let Ok(store) = CredentialStore::parse(text)
    {
        assert_eq!(store.render(), text);
        let _ = verify_bearer(Some(header), &store, 1000);
    }
});
