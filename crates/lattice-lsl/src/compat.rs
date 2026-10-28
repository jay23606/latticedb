//! Compatibility of a schema change with data that already exists.
//!
//! *Backward compatible*: every value valid under the old schema is valid under the new one, so
//! old data can be read after the change. *Forward compatible*: every value valid under the new
//! schema is valid under the old one, so old readers keep working.

use crate::ast::Ty;
use crate::check::{Compiled, Field};

/// Verdict for a schema change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compat {
    /// Old data stays valid.
    pub backward: bool,
    /// New data is valid for old readers.
    pub forward: bool,
    /// Why backward compatibility fails.
    pub backward_breaks: Vec<String>,
    /// Why forward compatibility fails.
    pub forward_breaks: Vec<String>,
}

/// Whether every value valid under `a` is also valid under `b` (so `b` accepts at least `a`).
fn accepts(a: &Ty, b: &Ty, ca: &Compiled, cb: &Compiled) -> bool {
    match (a, b) {
        (Ty::Bool, Ty::Bool) => true,
        (Ty::Int(ra), Ty::Int(rb)) => match (ra, rb) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some((la, ha)), Some((lb, hb))) => lb <= la && ha <= hb,
        },
        (Ty::Text(ma), Ty::Text(mb)) | (Ty::Bytes(ma), Ty::Bytes(mb)) => match (ma, mb) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(x), Some(y)) => x <= y,
        },
        (Ty::Ref(x), Ty::Ref(y)) => x == y,
        (Ty::List(ea, ma), Ty::List(eb, mb)) => {
            let max_ok = match (ma, mb) {
                (_, None) => true,
                (None, Some(_)) => false,
                (Some(x), Some(y)) => x <= y,
            };
            max_ok && accepts(ea, eb, ca, cb)
        }
        (Ty::Enum(x), Ty::Enum(y)) => match (ca.enums.get(x), cb.enums.get(y)) {
            (Some(va), Some(vb)) => va.iter().all(|v| vb.contains(v)),
            _ => false,
        },
        _ => false,
    }
}

/// Reasons data valid under `old` may be invalid under `new`.
fn breaks(old: &Compiled, new: &Compiled) -> Vec<String> {
    let mut out = Vec::new();
    for (tname, ofields) in &old.types {
        let Some(nfields) = new.types.get(tname) else {
            out.push(format!("type `{tname}` was removed"));
            continue;
        };
        for (
            fname,
            Field {
                ty: oty,
                optional: oopt,
            },
        ) in ofields
        {
            match nfields.get(fname) {
                None => out.push(format!(
                    "`{tname}.{fname}` was removed (existing data still carries it)"
                )),
                Some(Field {
                    ty: nty,
                    optional: nopt,
                }) => {
                    if !accepts(oty, nty, old, new) {
                        out.push(format!(
                            "`{tname}.{fname}` no longer accepts every old value"
                        ));
                    }
                    if *oopt && !nopt {
                        out.push(format!(
                            "`{tname}.{fname}` became required but old data may omit it"
                        ));
                    }
                }
            }
        }
        for (fname, Field { optional, .. }) in nfields {
            if !ofields.contains_key(fname) && !optional {
                out.push(format!(
                    "`{tname}.{fname}` is a new required field missing from old data"
                ));
            }
        }
    }
    out
}

/// Compares two compiled schemas.
pub fn compare(old: &Compiled, new: &Compiled) -> Compat {
    let backward_breaks = breaks(old, new);
    // Forward: every value valid under `new` is valid under `old` — the same question reversed,
    // except that types that exist only in `new` are irrelevant to old readers.
    let mut forward_breaks = Vec::new();
    for (tname, nfields) in &new.types {
        if !old.types.contains_key(tname) {
            // Old readers cannot validate values of a type they have never heard of.
            forward_breaks.push(format!(
                "type `{tname}` is new; old readers cannot validate it"
            ));
            continue;
        }
        let only: Compiled = Compiled {
            enums: new.enums.clone(),
            types: [(tname.clone(), nfields.clone())].into_iter().collect(),
        };
        forward_breaks.extend(breaks(&only, old));
    }
    Compat {
        backward: backward_breaks.is_empty(),
        forward: forward_breaks.is_empty(),
        backward_breaks,
        forward_breaks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::check;
    use crate::parser::parse;
    use crate::validate::{Value, validate};
    use std::collections::BTreeMap;

    fn c(src: &str) -> Compiled {
        check(&parse(src).unwrap()).unwrap()
    }

    fn verdict(old: &str, new: &str) -> (bool, bool) {
        let r = compare(&c(old), &c(new));
        (r.backward, r.forward)
    }

    #[test]
    fn classic_changes() {
        // Adding an optional field: old data still valid; old readers reject the new field.
        assert_eq!(
            verdict("type T { a: int }", "type T { a: int  b: text optional }"),
            (true, false)
        );
        // Adding a required field breaks old data; old readers cannot see it but data with it is
        // rejected by them as an unknown field.
        assert_eq!(
            verdict("type T { a: int }", "type T { a: int  b: text }"),
            (false, false)
        );
        // Making a field optional is backward compatible and not forward compatible.
        assert_eq!(
            verdict("type T { a: int }", "type T { a: int optional }"),
            (true, false)
        );
        // Widening a range: old data valid; new data may exceed what old readers allow.
        assert_eq!(
            verdict("type T { a: int in 0..=5 }", "type T { a: int in 0..=10 }"),
            (true, false)
        );
        // Narrowing is the reverse.
        assert_eq!(
            verdict("type T { a: int in 0..=10 }", "type T { a: int in 0..=5 }"),
            (false, true)
        );
        // Adding an enum variant is backward but not forward compatible.
        assert_eq!(
            verdict(
                "enum E { a }\ntype T { e: E }",
                "enum E { a, b }\ntype T { e: E }"
            ),
            (true, false)
        );
        // No change at all is both.
        assert_eq!(
            verdict("type T { a: int }", "type T { a: int }"),
            (true, true)
        );
        // A new type means old readers cannot validate its values.
        assert_eq!(
            verdict("type T { a: int }", "type T { a: int }\ntype U {}"),
            (true, false)
        );
        // Removing a type breaks backward compatibility.
        assert!(!verdict("type T { a: int }\ntype U {}", "type T { a: int }").0);
        // Changing a type is neither.
        assert_eq!(
            verdict("type T { a: int }", "type T { a: text }"),
            (false, false)
        );
    }

    /// If `compare` says backward compatible, a sample of old-valid values must validate under the
    /// new schema. This guards the checker against its own bugs.
    #[test]
    fn backward_verdict_is_sound_on_sampled_values() {
        let old = c("enum E { a, b }\ntype T { n: int in 0..=3  s: text max 2 optional  e: E }");
        let new = c(
            "enum E { a, b, c }\ntype T { n: int in 0..=9  s: text max 5 optional  e: E  extra: bool optional }",
        );
        assert!(compare(&old, &new).backward);
        for n in 0..=3 {
            for s in [None, Some(""), Some("ab")] {
                for e in ["a", "b"] {
                    let mut obj = BTreeMap::new();
                    obj.insert("n".to_string(), Value::Int(n));
                    obj.insert("e".to_string(), Value::Variant(e.into()));
                    if let Some(s) = s {
                        obj.insert("s".to_string(), Value::Text(s.into()));
                    }
                    let v = Value::Object(obj);
                    assert!(validate(&old, "T", &v).is_ok());
                    assert!(validate(&new, "T", &v).is_ok(), "{v:?}");
                }
            }
        }
    }
}
