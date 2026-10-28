#![no_main]
use lattice_lsl::check::check;
use lattice_lsl::parser::parse;
use lattice_lsl::validate::{Value, validate};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(schema) = parse(text)
        && let Ok(compiled) = check(&schema)
    {
        let empty = Value::Object(Default::default());
        for t in compiled.types.keys() {
            let _ = validate(&compiled, t, &empty);
        }
    }
});
