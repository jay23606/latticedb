//! Lexer and recursive-descent parser. Never panics on any input.

use crate::Diagnostic;
use crate::ast::{FieldDef, Item, Schema, Ty};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Int(i64),
    LBrace,
    RBrace,
    Lt,
    Gt,
    Colon,
    Eq,
    Plus,
    Comma,
    DotDot,
    DotDotEq,
}

struct Spanned {
    tok: Tok,
    line: u32,
    col: u32,
}

fn lex(src: &str) -> Result<Vec<Spanned>, Diagnostic> {
    let chars: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    let mut out = Vec::new();
    while let Some(&c) = chars.get(i) {
        let (l, cl) = (line, col);
        let adv = |n: usize, i: &mut usize, col: &mut u32| {
            *i += n;
            *col += n as u32;
        };
        if c == '\n' {
            i += 1;
            line += 1;
            col = 1;
        } else if c.is_whitespace() {
            adv(1, &mut i, &mut col);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while chars.get(i).is_some_and(|&x| x != '\n') {
                adv(1, &mut i, &mut col);
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while chars
                .get(i)
                .is_some_and(|&x| x.is_alphanumeric() || x == '_')
            {
                adv(1, &mut i, &mut col);
            }
            let text: String = chars.get(start..i).unwrap_or(&[]).iter().collect();
            out.push(Spanned {
                tok: Tok::Ident(text),
                line: l,
                col: cl,
            });
        } else if c.is_ascii_digit()
            || (c == '-' && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let start = i;
            adv(1, &mut i, &mut col);
            while chars.get(i).is_some_and(char::is_ascii_digit) {
                adv(1, &mut i, &mut col);
            }
            let text: String = chars.get(start..i).unwrap_or(&[]).iter().collect();
            let n = text
                .parse::<i64>()
                .map_err(|_| Diagnostic::new(l, cl, format!("number {text} is out of range")))?;
            out.push(Spanned {
                tok: Tok::Int(n),
                line: l,
                col: cl,
            });
        } else if c == '.' && chars.get(i + 1) == Some(&'.') {
            if chars.get(i + 2) == Some(&'=') {
                adv(3, &mut i, &mut col);
                out.push(Spanned {
                    tok: Tok::DotDotEq,
                    line: l,
                    col: cl,
                });
            } else {
                adv(2, &mut i, &mut col);
                out.push(Spanned {
                    tok: Tok::DotDot,
                    line: l,
                    col: cl,
                });
            }
        } else {
            let tok = match c {
                '{' => Tok::LBrace,
                '}' => Tok::RBrace,
                '<' => Tok::Lt,
                '>' => Tok::Gt,
                ':' => Tok::Colon,
                '=' => Tok::Eq,
                '+' => Tok::Plus,
                ',' => Tok::Comma,
                other => {
                    return Err(Diagnostic::new(
                        l,
                        cl,
                        format!("unexpected character {other:?}"),
                    ));
                }
            };
            adv(1, &mut i, &mut col);
            out.push(Spanned {
                tok,
                line: l,
                col: cl,
            });
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Spanned>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|s| &s.tok)
    }
    fn here(&self) -> (u32, u32) {
        self.toks
            .get(self.pos)
            .or_else(|| self.toks.last())
            .map_or((1, 1), |s| (s.line, s.col))
    }
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, Diagnostic> {
        let (l, c) = self.here();
        Err(Diagnostic::new(l, c, msg))
    }
    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|s| s.tok.clone());
        self.pos += 1;
        t
    }
    fn expect(&mut self, want: &Tok, what: &str) -> Result<(), Diagnostic> {
        if self.peek() == Some(want) {
            self.pos += 1;
            Ok(())
        } else {
            self.err(format!("expected {what}"))
        }
    }
    fn ident(&mut self, what: &str) -> Result<String, Diagnostic> {
        match self.peek() {
            Some(Tok::Ident(s)) => {
                let s = s.clone();
                self.pos += 1;
                Ok(s)
            }
            _ => self.err(format!("expected {what}")),
        }
    }
    fn eat_kw(&mut self, kw: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Ident(s)) if s == kw) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn uint(&mut self, what: &str) -> Result<u64, Diagnostic> {
        match self.peek() {
            Some(Tok::Int(n)) if *n >= 0 => {
                let n = *n as u64;
                self.pos += 1;
                Ok(n)
            }
            _ => self.err(format!("expected {what}")),
        }
    }
    fn int(&mut self, what: &str) -> Result<i64, Diagnostic> {
        match self.peek() {
            Some(Tok::Int(n)) => {
                let n = *n;
                self.pos += 1;
                Ok(n)
            }
            _ => self.err(format!("expected {what}")),
        }
    }

    fn ty(&mut self) -> Result<Ty, Diagnostic> {
        self.ty_at(0)
    }

    fn ty_at(&mut self, depth: usize) -> Result<Ty, Diagnostic> {
        if depth > MAX_TYPE_DEPTH {
            return self.err(format!("types nest more than {MAX_TYPE_DEPTH} levels deep"));
        }
        let name = self.ident("a type")?;
        match name.as_str() {
            "bool" => Ok(Ty::Bool),
            "int" => {
                if self.eat_kw("in") {
                    let lo = self.int("range start")?;
                    let inclusive = match self.bump() {
                        Some(Tok::DotDotEq) => true,
                        Some(Tok::DotDot) => false,
                        _ => return self.err("expected `..` or `..=`"),
                    };
                    let hi = self.int("range end")?;
                    let hi = if inclusive {
                        hi
                    } else {
                        match hi.checked_sub(1) {
                            Some(h) => h,
                            None => return self.err("empty range"),
                        }
                    };
                    Ok(Ty::Int(Some((lo, hi))))
                } else {
                    Ok(Ty::Int(None))
                }
            }
            "text" => Ok(Ty::Text(self.max_opt()?)),
            "bytes" => Ok(Ty::Bytes(self.max_opt()?)),
            "ref" => {
                self.expect(&Tok::Lt, "`<`")?;
                let target = self.ident("a type name")?;
                self.expect(&Tok::Gt, "`>`")?;
                Ok(Ty::Ref(target))
            }
            "list" => {
                self.expect(&Tok::Lt, "`<`")?;
                let elem = self.ty_at(depth + 1)?;
                self.expect(&Tok::Gt, "`>`")?;
                Ok(Ty::List(Box::new(elem), self.max_opt()?))
            }
            other => Ok(Ty::Enum(other.to_string())),
        }
    }

    fn max_opt(&mut self) -> Result<Option<u64>, Diagnostic> {
        if self.eat_kw("max") {
            Ok(Some(self.uint("a maximum")?))
        } else {
            Ok(None)
        }
    }

    fn fields(&mut self) -> Result<Vec<FieldDef>, Diagnostic> {
        self.expect(&Tok::LBrace, "`{`")?;
        let mut out = Vec::new();
        while self.peek() != Some(&Tok::RBrace) {
            if self.peek().is_none() {
                return self.err("unclosed `{`");
            }
            let (line, _) = self.here();
            let name = self.ident("a field name")?;
            self.expect(&Tok::Colon, "`:`")?;
            let ty = self.ty()?;
            let optional = self.eat_kw("optional");
            out.push(FieldDef {
                name,
                ty,
                optional,
                line,
            });
            if self.peek() == Some(&Tok::Comma) {
                self.pos += 1;
            }
        }
        self.pos += 1;
        Ok(out)
    }

    fn item(&mut self) -> Result<Item, Diagnostic> {
        let (line, _) = self.here();
        let kw = self.ident("`enum`, `facet` or `type`")?;
        match kw.as_str() {
            "enum" => {
                let name = self.ident("an enum name")?;
                self.expect(&Tok::LBrace, "`{`")?;
                let mut variants = Vec::new();
                while self.peek() != Some(&Tok::RBrace) {
                    if self.peek().is_none() {
                        return self.err("unclosed `{`");
                    }
                    variants.push(self.ident("a variant name")?);
                    if self.peek() == Some(&Tok::Comma) {
                        self.pos += 1;
                    }
                }
                self.pos += 1;
                Ok(Item::Enum {
                    name,
                    variants,
                    line,
                })
            }
            "facet" => {
                let name = self.ident("a facet name")?;
                Ok(Item::Facet {
                    name,
                    fields: self.fields()?,
                    line,
                })
            }
            "type" => {
                let name = self.ident("a type name")?;
                let mut facets = Vec::new();
                if self.peek() == Some(&Tok::Eq) {
                    self.pos += 1;
                    loop {
                        facets.push(self.ident("a facet name")?);
                        if self.peek() == Some(&Tok::Plus) {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                }
                Ok(Item::Type {
                    name,
                    facets,
                    fields: self.fields()?,
                    line,
                })
            }
            other => {
                self.pos = self.pos.saturating_sub(1);
                self.err(format!("unknown declaration `{other}`"))
            }
        }
    }
}

/// Largest schema text accepted, in bytes. Bounds the work an untrusted schema can cause.
pub const MAX_SCHEMA_BYTES: usize = 1 << 20;

/// Deepest allowed `list<...>` nesting. Bounds recursion depth.
pub const MAX_TYPE_DEPTH: usize = 8;

/// Parses schema text. Input size and type nesting are bounded, so no input can exhaust the stack.
pub fn parse(src: &str) -> Result<Schema, Diagnostic> {
    if src.len() > MAX_SCHEMA_BYTES {
        return Err(Diagnostic::new(
            1,
            1,
            format!("schema is larger than {MAX_SCHEMA_BYTES} bytes"),
        ));
    }
    let mut p = Parser {
        toks: lex(src)?,
        pos: 0,
    };
    let mut items = Vec::new();
    while p.peek().is_some() {
        items.push(p.item()?);
    }
    Ok(Schema { items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parses_every_construct() {
        let s = parse(
            "// comment\nenum P { low, high }\nfacet E { n: int in 0..=100 optional  p: P }\ntype T = E { t: text max 9, r: ref<U>, l: list<bytes max 4> max 3, b: bool }\ntype U {}\n",
        )
        .unwrap();
        assert_eq!(s.items.len(), 4);
    }

    #[test]
    fn diagnostics_are_located_and_stable() {
        let e = parse("type T {\n  x text\n}").unwrap_err();
        assert_eq!((e.line, e.col, e.message.as_str()), (2, 5, "expected `:`"));
        assert_eq!(
            parse("type T { x: int in 5 }").unwrap_err().message,
            "expected `..` or `..=`"
        );
        assert_eq!(
            parse("type T { x: text").unwrap_err().message,
            "unclosed `{`"
        );
        assert_eq!(
            parse("widget W {}").unwrap_err().message,
            "unknown declaration `widget`"
        );
        assert_eq!(
            parse("type T { x: text # }").unwrap_err().message,
            "unexpected character '#'"
        );
    }

    #[test]
    fn hostile_inputs_are_rejected_not_crashed_on() {
        let deep = format!(
            "type T {{ x: {}int{} }}",
            "list<".repeat(100_000),
            ">".repeat(100_000)
        );
        assert!(parse(&deep).unwrap_err().message.contains("nest"));
        let huge = format!("// {}", "a".repeat(MAX_SCHEMA_BYTES));
        assert!(parse(&huge).unwrap_err().message.contains("larger than"));
        let empty = parse("type T { x: int in -9223372036854775808..-9223372036854775808 }");
        assert_eq!(empty.unwrap_err().message, "empty range");
        assert!(parse("type T { x: int in 0..=5 }").is_ok());
    }

    proptest! {
        #[test]
        fn arbitrary_text_never_panics(src in ".{0,200}") {
            let _ = parse(&src);
        }

        #[test]
        fn token_soup_never_panics(words in proptest::collection::vec(prop_oneof![
            Just("type"), Just("facet"), Just("enum"), Just("{"), Just("}"), Just("<"), Just(">"),
            Just(":"), Just("="), Just("+"), Just(","), Just("int"), Just("text"), Just("max"),
            Just("in"), Just("0..=5"), Just("optional"), Just("ref"), Just("list"), Just("x"),
        ], 0..40)) {
            let _ = parse(&words.join(" "));
        }
    }
}
