//! Minimal JSON parser (enough for catalog APIs like Gutendex). No serde.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Json {
    Null,
    #[allow(dead_code)] // parsed for completeness; no caller needs booleans yet
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, k: &str) -> &Json {
        static NULL: Json = Json::Null;
        match self {
            Json::Obj(m) => m.get(k).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(v) => v,
            _ => &[],
        }
    }
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.b.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                loop {
                    self.ws();
                    if self.b.get(self.i) == Some(&b'}') {
                        self.i += 1;
                        return Ok(Json::Obj(m));
                    }
                    let k = match self.value()? {
                        Json::Str(s) => s,
                        _ => return Err("object key must be a string".into()),
                    };
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return Err(format!("expected ':' at {}", self.i));
                    }
                    self.i += 1;
                    let v = self.value()?;
                    m.insert(k, v);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {}
                        _ => return Err(format!("expected ',' or '}}' at {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut v = vec![];
                loop {
                    self.ws();
                    if self.b.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Ok(Json::Arr(v));
                    }
                    v.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {}
                        _ => return Err(format!("expected ',' or ']' at {}", self.i)),
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                let mut s = String::new();
                loop {
                    let c = *self.b.get(self.i).ok_or("unterminated string")?;
                    self.i += 1;
                    match c {
                        b'"' => return Ok(Json::Str(s)),
                        b'\\' => {
                            let e = *self.b.get(self.i).ok_or("bad escape")?;
                            self.i += 1;
                            match e {
                                b'n' => s.push('\n'),
                                b't' => s.push('\t'),
                                b'r' => s.push('\r'),
                                b'b' | b'f' => {}
                                b'u' => {
                                    let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4).ok_or("bad \\u")?)
                                        .map_err(|_| "bad \\u")?;
                                    let mut cp = u32::from_str_radix(hex, 16).map_err(|_| "bad \\u")?;
                                    self.i += 4;
                                    // surrogate pair
                                    if (0xD800..0xDC00).contains(&cp) && self.b.get(self.i) == Some(&b'\\')
                                        && self.b.get(self.i + 1) == Some(&b'u') {
                                        if let Some(lo) = std::str::from_utf8(&self.b[self.i + 2..(self.i + 6).min(self.b.len())])
                                            .ok().and_then(|h| u32::from_str_radix(h, 16).ok()) {
                                            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                                            self.i += 6;
                                        }
                                    }
                                    s.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                                }
                                other => s.push(other as char),
                            }
                        }
                        _ => {
                            // copy a run of raw UTF-8 bytes
                            let start = self.i - 1;
                            while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' {
                                self.i += 1;
                            }
                            s.push_str(&String::from_utf8_lossy(&self.b[start..self.i]));
                        }
                    }
                }
            }
            Some(b't') if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(Json::Bool(true))
            }
            Some(b'f') if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(Json::Bool(false))
            }
            Some(b'n') if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Ok(Json::Null)
            }
            Some(_) => {
                let s = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.b[s..self.i]).ok().and_then(|t| t.parse().ok()).map(Json::Num)
                    .ok_or_else(|| format!("unexpected character at {s}"))
            }
            None => Err("unexpected end of JSON".into()),
        }
    }
}

pub fn parse(s: &str) -> Result<Json, String> {
    P { b: s.as_bytes(), i: 0 }.value()
}
