//! A tiny Rust lexer: blanks out comments and string or char literals so that token searches
//! (such as `unsafe` or `disallowed_`) only see real code. Newlines are preserved.

/// Returns `src` with comments and literals replaced by spaces.
pub fn code_only(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    while i < b.len() {
        let c = b[i];
        let next = b.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < b.len() && b[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == '/' && b.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if b[i] == '*' && b.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(b[i]));
                    i += 1;
                }
            }
        } else if let Some(skip) = raw_string_len(&b, i) {
            for k in 0..skip {
                out.push(blank(b[i + k]));
            }
            i += skip;
        } else if c == '"' || (c == 'b' && next == Some('"')) {
            if c == 'b' {
                out.push(' ');
                i += 1;
            }
            out.push(' ');
            i += 1;
            while i < b.len() && b[i] != '"' {
                if b[i] == '\\' && i + 1 < b.len() {
                    out.push(' ');
                    i += 1;
                }
                out.push(blank(b[i]));
                i += 1;
            }
            out.push(' ');
            i += 1;
        } else if c == '\'' {
            if next == Some('\\') {
                let mut j = i + 2;
                while j < b.len() && b[j] != '\'' {
                    j += 1;
                }
                for &ch in &b[i..=j.min(b.len() - 1)] {
                    out.push(blank(ch));
                }
                i = j + 1;
            } else if b.get(i + 2) == Some(&'\'') {
                out.push_str("   ");
                i += 3;
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Length of a raw string literal starting at `i` (`r"..."`, `r#"..."#`, `br#"..."#`), if any.
fn raw_string_len(b: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    if b.get(j) == Some(&'b') {
        j += 1;
    }
    if b.get(j) != Some(&'r') {
        return None;
    }
    // `r` must start a token, not end an identifier such as `for`.
    if i > 0 && (b[i - 1].is_alphanumeric() || b[i - 1] == '_') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while b.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if b.get(j) != Some(&'"') {
        return None;
    }
    j += 1;
    while j < b.len() {
        if b[j] == '"' && (0..hashes).all(|k| b.get(j + 1 + k) == Some(&'#')) {
            return Some(j + 1 + hashes - i);
        }
        j += 1;
    }
    Some(b.len() - i)
}

/// Whether `word` appears in `code` as a whole identifier.
#[cfg(test)]
pub fn has_word(code: &str, word: &str) -> bool {
    count_word(code, word) > 0
}

/// Number of whole-identifier occurrences of `word` in `code`.
pub fn count_word(code: &str, word: &str) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(pos) = code[from..].find(word) {
        let start = from + pos;
        let end = start + word.len();
        let before = code[..start].chars().next_back();
        let after = code[end..].chars().next();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !ident(before) && !ident(after) {
            n += 1;
        }
        from = end;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_and_comments_are_blanked() {
        let src = "let u = \"unsafe // not code\"; // unsafe in comment\n/* unsafe\n nested /* unsafe */ */ real";
        let code = code_only(src);
        assert!(!has_word(&code, "unsafe"));
        assert!(has_word(&code, "real"));
        assert!(has_word(&code, "let"));
    }

    #[test]
    fn code_after_a_url_string_is_still_seen() {
        let code = code_only("let u = \"https://x\"; unsafe { f() }");
        assert!(has_word(&code, "unsafe"));
    }

    #[test]
    fn raw_strings_and_chars_and_lifetimes() {
        let code = code_only(
            "let a = r#\"unsafe \" still\"#; let c = 'x'; fn f<'a>(x: &'a str) {} unsafe fn g() {}",
        );
        assert_eq!(count_word(&code, "unsafe"), 1);
        assert!(has_word(&code, "a"));
    }

    #[test]
    fn newline_and_tab_separated_unsafe_is_found() {
        assert!(has_word(&code_only("unsafe\n{ }"), "unsafe"));
        assert!(has_word(&code_only("x = unsafe\t{ }"), "unsafe"));
        assert!(!has_word(
            &code_only("let unsafety = 1; let unsafe_code = 2;"),
            "unsafe"
        ));
    }
}
