//! The type checker: resolves names, rejects conflicts and nonsense refinements, and flattens
//! composed facets into per-type field tables.

use crate::Diagnostic;
use crate::ast::{FieldDef, Item, Schema, Ty};
use std::collections::{BTreeMap, BTreeSet};

/// A checked field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The field type.
    pub ty: Ty,
    /// Whether the field may be absent.
    pub optional: bool,
}

/// A checked schema: facets are flattened away.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Compiled {
    /// Enum name to variants.
    pub enums: BTreeMap<String, Vec<String>>,
    /// Type name to field table.
    pub types: BTreeMap<String, BTreeMap<String, Field>>,
}

fn d(line: u32, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new(line, 1, msg)
}

fn check_ty(
    ty: &Ty,
    line: u32,
    types: &BTreeSet<&str>,
    enums: &BTreeSet<&str>,
    out: &mut Vec<Diagnostic>,
) {
    match ty {
        Ty::Int(Some((lo, hi))) if lo > hi => out.push(d(line, format!("empty range {lo}..={hi}"))),
        Ty::Text(Some(0)) | Ty::Bytes(Some(0)) | Ty::List(_, Some(0)) => {
            out.push(d(line, "`max 0` makes the field impossible to fill"));
        }
        Ty::Ref(t) if !types.contains(t.as_str()) => {
            out.push(d(
                line,
                format!("`ref<{t}>` names no type (refs must point at a `type`)"),
            ));
        }
        Ty::Enum(e) if !enums.contains(e.as_str()) => {
            out.push(d(line, format!("unknown type or enum `{e}`")))
        }
        Ty::List(inner, _) => {
            if matches!(**inner, Ty::List(..)) {
                out.push(d(line, "nested lists are not supported in v0"));
            }
            check_ty(inner, line, types, enums, out);
        }
        _ => {}
    }
}

/// Checks a schema. Returns every problem found, not just the first.
pub fn check(schema: &Schema) -> Result<Compiled, Vec<Diagnostic>> {
    let mut out = Vec::new();
    let mut names: BTreeMap<&str, u32> = BTreeMap::new();
    for item in &schema.items {
        let (n, line) = match item {
            Item::Enum { name, line, .. }
            | Item::Facet { name, line, .. }
            | Item::Type { name, line, .. } => (name.as_str(), *line),
        };
        if let Some(first) = names.insert(n, line) {
            out.push(d(
                line,
                format!("`{n}` is already declared on line {first}"),
            ));
        }
    }
    let types: BTreeSet<&str> = schema
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Type { name, .. } = i {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();
    let enums: BTreeSet<&str> = schema
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Enum { name, .. } = i {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();
    let facets: BTreeMap<&str, &Vec<FieldDef>> = schema
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Facet { name, fields, .. } = i {
                Some((name.as_str(), fields))
            } else {
                None
            }
        })
        .collect();

    let mut compiled = Compiled::default();
    for item in &schema.items {
        match item {
            Item::Enum {
                name,
                variants,
                line,
            } => {
                if variants.is_empty() {
                    out.push(d(*line, format!("enum `{name}` has no variants")));
                }
                let mut seen = BTreeSet::new();
                for v in variants {
                    if !seen.insert(v) {
                        out.push(d(*line, format!("enum `{name}` repeats variant `{v}`")));
                    }
                }
                compiled.enums.insert(name.clone(), variants.clone());
            }
            Item::Facet { fields, .. } => {
                let mut seen = BTreeSet::new();
                for f in fields {
                    if !seen.insert(&f.name) {
                        out.push(d(f.line, format!("field `{}` is declared twice", f.name)));
                    }
                    check_ty(&f.ty, f.line, &types, &enums, &mut out);
                }
            }
            Item::Type {
                name,
                facets: used,
                fields,
                line,
            } => {
                let mut table: BTreeMap<String, Field> = BTreeMap::new();
                let mut add = |f: &FieldDef, origin: &str, out: &mut Vec<Diagnostic>| {
                    if table.contains_key(&f.name) {
                        out.push(d(f.line, format!("field `{}` of {origin} conflicts with an existing field of type `{name}`", f.name)));
                    } else {
                        table.insert(
                            f.name.clone(),
                            Field {
                                ty: f.ty.clone(),
                                optional: f.optional,
                            },
                        );
                    }
                };
                for fname in used {
                    match facets.get(fname.as_str()) {
                        Some(fs) => fs
                            .iter()
                            .for_each(|f| add(f, &format!("facet `{fname}`"), &mut out)),
                        None => out.push(d(
                            *line,
                            format!("type `{name}` composes unknown facet `{fname}`"),
                        )),
                    }
                }
                for f in fields {
                    check_ty(&f.ty, f.line, &types, &enums, &mut out);
                    add(f, "the type itself", &mut out);
                }
                compiled.types.insert(name.clone(), table);
            }
        }
    }
    if out.is_empty() {
        Ok(compiled)
    } else {
        Err(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn check_text(s: &str) -> Result<Compiled, Vec<Diagnostic>> {
        check(&parse(s).unwrap())
    }

    fn messages(s: &str) -> Vec<String> {
        check_text(s)
            .unwrap_err()
            .into_iter()
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn composition_flattens_facets() {
        let c =
            check_text("enum P { a }\nfacet F { p: P  n: int }\ntype T = F { t: text }\n").unwrap();
        let fields: Vec<&String> = c.types["T"].keys().collect();
        assert_eq!(fields, ["n", "p", "t"]);
    }

    #[test]
    fn rejects_each_class_of_mistake() {
        assert!(
            messages("type T { x: int }\ntype T { y: int }")
                .iter()
                .any(|m| m.contains("already declared"))
        );
        assert!(
            messages("type T { x: ref<Nope> }")
                .iter()
                .any(|m| m.contains("names no type"))
        );
        assert!(
            messages("type T { x: Nope }")
                .iter()
                .any(|m| m.contains("unknown type or enum"))
        );
        assert!(
            messages("type T = Nope { x: int }")
                .iter()
                .any(|m| m.contains("unknown facet"))
        );
        assert!(
            messages("facet F { x: int }\ntype T = F { x: text }")
                .iter()
                .any(|m| m.contains("conflicts"))
        );
        assert!(
            messages("facet F { x: int }\nfacet G { x: int }\ntype T = F + G {}")
                .iter()
                .any(|m| m.contains("conflicts"))
        );
        assert!(
            messages("type T { x: int in 5..=1 }")
                .iter()
                .any(|m| m.contains("empty range"))
        );
        assert!(
            messages("type T { x: text max 0 }")
                .iter()
                .any(|m| m.contains("max 0"))
        );
        assert!(
            messages("enum E {}")
                .iter()
                .any(|m| m.contains("no variants"))
        );
        assert!(
            messages("enum E { a, a }")
                .iter()
                .any(|m| m.contains("repeats"))
        );
        assert!(
            messages("type T { x: list<list<int>> }")
                .iter()
                .any(|m| m.contains("nested"))
        );
        assert!(
            messages("enum E { a }\ntype T { r: ref<E> }")
                .iter()
                .any(|m| m.contains("names no type"))
        );
    }

    #[test]
    fn reports_all_problems_not_just_the_first() {
        assert!(messages("type T { a: Nope  b: Nope2 }").len() >= 2);
    }
}
