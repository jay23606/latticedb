#![no_main]
use lattice_engines::catalog;
use lattice_engines::sqldef::{self, SqlTableDef};
use libfuzzer_sys::fuzz_target;

// The first byte selects what to decode: 0 mod 3 for one definition body, 1 mod 3 for a definition
// block, 2 mod 3 for a whole LCIX envelope (version 1 or 3).
fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };
    if selector % 3 == 2 {
        if let Ok(change) = catalog::decode_compound(payload) {
            let encoded =
                catalog::encode_compound(&change).expect("a decoded envelope must be encodable");
            assert_eq!(
                encoded, payload,
                "successful decoding must preserve canonical bytes"
            );
            assert_eq!(catalog::decode_compound(&encoded), Ok(change));
        }
    } else if selector % 3 == 0 {
        if let Ok(def) = SqlTableDef::decode(payload) {
            let encoded = def.encode().expect("a decoded definition must be encodable");
            assert_eq!(
                encoded, payload,
                "successful decoding must preserve canonical bytes"
            );
            assert_eq!(SqlTableDef::decode(&encoded), Ok(def));
        }
    } else if let Ok(ops) = sqldef::decode_ops(payload) {
        let encoded = sqldef::encode_ops(&ops).expect("decoded definitions must be encodable");
        assert_eq!(
            encoded, payload,
            "successful decoding must preserve canonical bytes"
        );
    }
});
