//! Cursor store tests: the stored form, every refusal, and the in-memory store through the trait.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use lattice_sdk::cursor_store::{
    CursorStore, MalformedCursor, MemoryCursorStore, decode_cursor, encode_cursor,
};
use lattice_sdk::mcp::{CURSOR_LEN, FeedCursor};

const LOW: FeedCursor = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const LOW_TEXT: &str = "000102030405060708090a0b";
const HIGH: FeedCursor = [0xff; CURSOR_LEN];
const HIGH_TEXT: &str = "ffffffffffffffffffffffff";

#[test]
fn encode_writes_24_lowercase_digits_and_one_newline() {
    assert_eq!(encode_cursor(&LOW), "000102030405060708090a0b\n");
    assert_eq!(encode_cursor(&HIGH), "ffffffffffffffffffffffff\n");
    assert_eq!(encode_cursor(&LOW).len(), 25);
}

#[test]
fn decode_accepts_the_canonical_form_with_or_without_the_newline() {
    assert_eq!(decode_cursor(b"000102030405060708090a0b\n"), Ok(LOW));
    assert_eq!(decode_cursor(LOW_TEXT.as_bytes()), Ok(LOW));
    assert_eq!(decode_cursor(HIGH_TEXT.as_bytes()), Ok(HIGH));
}

#[test]
fn round_trip_returns_every_cursor_it_was_given() {
    let samples: [FeedCursor; 4] = [
        LOW,
        HIGH,
        [0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ];
    for cursor in samples {
        assert_eq!(decode_cursor(encode_cursor(&cursor).as_bytes()), Ok(cursor));
    }
}

#[test]
fn decode_refuses_a_wrong_length() {
    assert_eq!(decode_cursor(b""), Err(MalformedCursor));
    assert_eq!(decode_cursor(b"\n"), Err(MalformedCursor));
    assert_eq!(
        decode_cursor(b"000102030405060708090a0"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"000102030405060708090a0b0"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"000102030405060708090a0b0\n"),
        Err(MalformedCursor)
    );
}

#[test]
fn decode_refuses_uppercase_and_non_hex_digits() {
    assert_eq!(
        decode_cursor(b"000102030405060708090A0B\n"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"000102030405060708090a0B\n"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"0001020304050607080g0b\n"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"0001020304050607080 0b\n"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"0001020304050607080-0b\n"),
        Err(MalformedCursor)
    );
}

#[test]
fn decode_refuses_any_byte_after_the_one_optional_newline() {
    assert_eq!(
        decode_cursor(b"000102030405060708090a0b\n\n"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"000102030405060708090a0b\r\n"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"000102030405060708090a0b\nx"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"000102030405060708090a0b \n"),
        Err(MalformedCursor)
    );
    assert_eq!(
        decode_cursor(b"000102030405060708090a0b\0"),
        Err(MalformedCursor)
    );
}

#[test]
fn decode_refuses_every_torn_prefix_of_a_saved_cursor() {
    let saved = encode_cursor(&LOW).into_bytes();
    // A cut after the 24th digit is the complete no-newline form, which is accepted by design;
    // the file store's atomic rename is what keeps a torn file from being read at all.
    for cut in 0..CURSOR_LEN * 2 {
        assert_eq!(
            decode_cursor(&saved[..cut]),
            Err(MalformedCursor),
            "prefix of {cut} bytes"
        );
    }
    assert_eq!(decode_cursor(&saved[..CURSOR_LEN * 2]), Ok(LOW));
}

#[test]
fn decode_never_panics_and_accepts_only_canonical_spellings() {
    const ALPHABET: &[u8] = b"0123456789abcdefABCDEF\n \x00\xff";
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        seed >> 33
    };
    for len in 0..40 {
        for _ in 0..200 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| ALPHABET[(next() as usize) % ALPHABET.len()])
                .collect();
            if let Ok(cursor) = decode_cursor(&bytes) {
                let canonical = encode_cursor(&cursor).into_bytes();
                assert!(
                    bytes == canonical || bytes == canonical[..CURSOR_LEN * 2],
                    "accepted a non-canonical spelling: {bytes:?}"
                );
            }
        }
    }
}

#[test]
fn fixed_fill_inputs_are_accepted_only_at_the_canonical_length() {
    for len in 0..40 {
        for fill in [b'0', b'a', b'A', b'g', b'\n', 0x00, 0xff] {
            let bytes = vec![fill; len];
            let expected_ok = len == 24 && (fill == b'0' || fill == b'a');
            assert_eq!(
                decode_cursor(&bytes).is_ok(),
                expected_ok,
                "len {len}, fill {fill:#04x}"
            );
        }
    }
}

#[test]
fn memory_store_starts_empty_and_round_trips_through_the_trait() {
    fn round_trip<S: CursorStore<Error = MalformedCursor>>(store: &mut S) {
        assert_eq!(store.load(), Ok(None));
        store.save(&LOW).unwrap();
        assert_eq!(store.load(), Ok(Some(LOW)));
        store.save(&HIGH).unwrap();
        assert_eq!(store.load(), Ok(Some(HIGH)));
    }
    let mut store = MemoryCursorStore::new();
    round_trip(&mut store);
    assert_eq!(store.bytes(), Some(&b"ffffffffffffffffffffffff\n"[..]));
}

#[test]
fn memory_store_reports_stored_bytes_that_are_not_canonical() {
    let upper = MemoryCursorStore::with_bytes(b"000102030405060708090A0B");
    assert_eq!(upper.load(), Err(MalformedCursor));
    let torn = MemoryCursorStore::with_bytes(b"0001020304050607");
    assert_eq!(torn.load(), Err(MalformedCursor));
}

#[test]
fn a_save_replaces_malformed_stored_bytes() {
    let mut store = MemoryCursorStore::with_bytes(b"not a cursor");
    assert_eq!(store.load(), Err(MalformedCursor));
    store.save(&LOW).unwrap();
    assert_eq!(store.load(), Ok(Some(LOW)));
    assert_eq!(store.bytes(), Some(&b"000102030405060708090a0b\n"[..]));
}

#[test]
fn malformed_cursor_has_a_fixed_message() {
    assert_eq!(MalformedCursor.to_string(), "stored cursor is malformed");
}
