#![no_main]
use lattice_engines::sqldef::{self, SqlTableDef};
use libfuzzer_sys::fuzz_target;

// The first byte selects what to decode: even for one definition body, odd for a definition block.
fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };
    if selector % 2 == 0 {
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
