//! A strict, small JSON reader for the test clients. The replies of `POST /mcp` are judged from their
//! bytes, so the harness reads them itself rather than through the server's own types.

/// A parsed JSON value. A number keeps its source text, so the caller chooses how to read it.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, as written.
    Number(String),
    /// A string, with its escapes decoded.
    String(String),
    /// An array, in order.
    Array(Vec<Json>),
    /// An object, members in order. A repeated key is an error.
    Object(Vec<(String, Json)>),
}

/// The deepest nesting the reader accepts.
const MAX_DEPTH: usize = 32;

impl Json {
    /// Parses one complete JSON text. Bytes after the value are an error.
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut reader = Reader {
            bytes: text.as_bytes(),
            at: 0,
        };
        let value = reader.value(0)?;
        reader.skip_space();
        if reader.at == reader.bytes.len() {
            Ok(value)
        } else {
            reader.fail("bytes after the value")
        }
    }

    /// The member `key` of an object, when this value is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members
                .iter()
                .find(|(name, _)| name.as_str() == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The text of a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text.as_str()),
            _ => None,
        }
    }

    /// The elements of an array.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    /// The value of a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    /// The value of a number that is an integer in the range of `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Number(text) => text.parse().ok(),
            _ => None,
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("{what} at byte {}", self.at))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.peek() == Some(byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn digits(&mut self) -> usize {
        let mut count = 0;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
            count += 1;
        }
        count
    }

    fn word(&mut self, word: &str, value: Json) -> Result<Json, String> {
        let end = self.at + word.len();
        if self.bytes.get(self.at..end) == Some(word.as_bytes()) {
            self.at = end;
            Ok(value)
        } else {
            self.fail("unknown literal")
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return self.fail("nesting too deep");
        }
        self.skip_space();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::String),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => self.fail("expected a value"),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.at += 1;
        let mut members: Vec<(String, Json)> = Vec::new();
        self.skip_space();
        if self.eat(b'}') {
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return self.fail("expected a key");
            }
            let key = self.string()?;
            if members.iter().any(|(name, _)| *name == key) {
                return self.fail("repeated key");
            }
            self.skip_space();
            if !self.eat(b':') {
                return self.fail("expected ':'");
            }
            let value = self.value(depth + 1)?;
            members.push((key, value));
            self.skip_space();
            if self.eat(b'}') {
                return Ok(Json::Object(members));
            }
            if !self.eat(b',') {
                return self.fail("expected ',' or '}'");
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.eat(b']') {
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_space();
            if self.eat(b']') {
                return Ok(Json::Array(items));
            }
            if !self.eat(b',') {
                return self.fail("expected ',' or ']'");
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(byte) = self.peek() else {
                return self.fail("unterminated string");
            };
            self.at += 1;
            match byte {
                b'"' => {
                    return match String::from_utf8(out) {
                        Ok(text) => Ok(text),
                        Err(_) => self.fail("invalid UTF-8"),
                    };
                }
                b'\\' => {
                    let Some(escape) = self.peek() else {
                        return self.fail("unterminated escape");
                    };
                    self.at += 1;
                    let ch = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode()?,
                        _ => return self.fail("unknown escape"),
                    };
                    let mut utf8 = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut utf8).as_bytes());
                }
                0x00..=0x1f => return self.fail("control character in a string"),
                _ => out.push(byte),
            }
        }
    }

    fn unicode(&mut self) -> Result<char, String> {
        let high = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&high) {
            if !(self.eat(b'\\') && self.eat(b'u')) {
                return self.fail("lone surrogate");
            }
            let low = self.hex4()?;
            if !(0xDC00..0xE000).contains(&low) {
                return self.fail("lone surrogate");
            }
            0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
        } else {
            high
        };
        match char::from_u32(code) {
            Some(ch) => Ok(ch),
            None => self.fail("lone surrogate"),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let bytes = self.bytes;
        let Some(digits) = bytes.get(self.at..self.at + 4) else {
            return self.fail("short \\u escape");
        };
        let mut value = 0u32;
        for &byte in digits {
            let Some(digit) = (byte as char).to_digit(16) else {
                return self.fail("bad \\u escape");
            };
            value = value * 16 + digit;
        }
        self.at += 4;
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        self.eat(b'-');
        // JSON allows a single zero before the point or exponent, never leading zeros such as `01`.
        if self.eat(b'0') {
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                return self.fail("leading zero");
            }
        } else if self.digits() == 0 {
            return self.fail("expected digits");
        }
        if self.eat(b'.') && self.digits() == 0 {
            return self.fail("expected digits after '.'");
        }
        if self.eat(b'e') || self.eat(b'E') {
            let _ = self.eat(b'+') || self.eat(b'-');
            if self.digits() == 0 {
                return self.fail("expected digits in the exponent");
            }
        }
        let text = self.bytes.get(start..self.at).unwrap_or_default();
        Ok(Json::Number(String::from_utf8_lossy(text).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::Json;

    #[test]
    fn reads_nested_values_and_decodes_escapes() {
        let text = r#" {"a":[1,-2.5e3,true,null],"b":"x\"é😀"} "#;
        let value = Json::parse(text).unwrap();
        let items = value.get("a").and_then(Json::as_array).unwrap();
        assert_eq!(items[0].as_i64(), Some(1));
        assert_eq!(items[2].as_bool(), Some(true));
        assert_eq!(
            value.get("b").and_then(Json::as_str),
            Some("x\"\u{e9}\u{1F600}")
        );
    }

    #[test]
    fn refuses_trailing_bytes_repeated_keys_and_bad_escapes() {
        assert!(Json::parse("{} x").is_err());
        assert!(Json::parse(r#"{"k":1,"k":2}"#).is_err());
        assert!(Json::parse(r#""\x""#).is_err());
        assert!(Json::parse(r#""\ud800""#).is_err());
        assert!(Json::parse("[1,]").is_err());
    }

    #[test]
    fn refuses_leading_zeros_and_accepts_a_lone_zero() {
        assert!(Json::parse("01").is_err());
        assert!(Json::parse("-01").is_err());
        assert!(Json::parse("[00]").is_err());
        assert!(Json::parse("0").is_ok());
        assert!(Json::parse("-0").is_ok());
        assert!(Json::parse("0.5").is_ok());
        assert!(Json::parse("-0.5e1").is_ok());
    }
}
