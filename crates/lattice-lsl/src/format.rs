//! The canonical formatter. `parse(format(x))` equals `x`, which the tests check.

use crate::ast::{Item, Schema, Ty};

fn ty(t: &Ty) -> String {
    match t {
        Ty::Bool => "bool".into(),
        Ty::Int(None) => "int".into(),
        Ty::Int(Some((lo, hi))) => format!("int in {lo}..={hi}"),
        Ty::Text(m) => max("text", *m),
        Ty::Bytes(m) => max("bytes", *m),
        Ty::Ref(n) => format!("ref<{n}>"),
        Ty::List(inner, m) => max(&format!("list<{}>", ty(inner)), *m),
        Ty::Enum(n) => n.clone(),
    }
}

fn max(base: &str, m: Option<u64>) -> String {
    m.map_or_else(|| base.to_string(), |m| format!("{base} max {m}"))
}

/// Formats a schema canonically.
pub fn format(s: &Schema) -> String {
    let mut out = String::new();
    for item in &s.items {
        match item {
            Item::Enum { name, variants, .. } => {
                out.push_str(&format!("enum {name} {{ {} }}\n", variants.join(", ")))
            }
            Item::Facet { name, fields, .. } => {
                out.push_str(&format!("facet {name} {{\n"));
                for f in fields {
                    out.push_str(&format!(
                        "  {}: {}{}\n",
                        f.name,
                        ty(&f.ty),
                        if f.optional { " optional" } else { "" }
                    ));
                }
                out.push_str("}\n");
            }
            Item::Type {
                name,
                facets,
                fields,
                ..
            } => {
                let comp = if facets.is_empty() {
                    String::new()
                } else {
                    format!(" = {}", facets.join(" + "))
                };
                out.push_str(&format!("type {name}{comp} {{\n"));
                for f in fields {
                    out.push_str(&format!(
                        "  {}: {}{}\n",
                        f.name,
                        ty(&f.ty),
                        if f.optional { " optional" } else { "" }
                    ));
                }
                out.push_str("}\n");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn strip_lines(s: Schema) -> Schema {
        // Line numbers differ between the original and formatted text; compare structure only.
        let items = s
            .items
            .into_iter()
            .map(|i| match i {
                Item::Enum { name, variants, .. } => Item::Enum {
                    name,
                    variants,
                    line: 0,
                },
                Item::Facet {
                    name, mut fields, ..
                } => {
                    fields.iter_mut().for_each(|f| f.line = 0);
                    Item::Facet {
                        name,
                        fields,
                        line: 0,
                    }
                }
                Item::Type {
                    name,
                    facets,
                    mut fields,
                    ..
                } => {
                    fields.iter_mut().for_each(|f| f.line = 0);
                    Item::Type {
                        name,
                        facets,
                        fields,
                        line: 0,
                    }
                }
            })
            .collect();
        Schema { items }
    }

    #[test]
    fn format_round_trips_and_is_idempotent() {
        let src = "enum P{low,high} facet E{n:int in 0..=100 optional p:P} type T=E{t:text max 9 r:ref<U> l:list<bytes max 4> max 3 b:bool} type U{}";
        let a = parse(src).unwrap();
        let text = format(&a);
        let b = parse(&text).unwrap();
        assert_eq!(strip_lines(a), strip_lines(b.clone()));
        assert_eq!(format(&b), text, "formatting must be idempotent");
    }
}
