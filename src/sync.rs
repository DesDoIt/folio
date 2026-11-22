//! Reading-progress sync with a KOReader sync server ("kosync"; default: KOReader's public server).
//! Books are identified like KOReader does: MD5 over 1 KB samples at growing offsets.
//! Positions sync per chapter (as a DocFragment xpointer) plus the book percentage.

use std::process::{Command, Stdio};

pub const DEFAULT_SERVER: &str = "https://sync.koreader.rocks";

// --- MD5 (RFC 1321) --------------------------------------------------------------------------------
pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
        14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6,
        10, 15, 21, 6, 10, 15, 21];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in msg.chunks(64) {
        let m: Vec<u32> = chunk.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f2 = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f2.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

pub fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// KOReader's "partial MD5" document fingerprint: 1 KB samples at offsets 1024 << (2*i), i = -1..10.
/// (In KOReader's LuaJIT, a shift by -2 wraps to 30 and overflows to 0, so the first sample is at 0.)
pub fn document_id(path: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut buf = Vec::new();
    for i in -1i32..=10 {
        let offset: u64 = if i < 0 { 0 } else { 1024u64 << (2 * i) };
        if offset >= len {
            break;
        }
        f.seek(SeekFrom::Start(offset)).ok()?;
        let mut chunk = vec![0u8; 1024];
        let n = f.read(&mut chunk).ok()?;
        buf.extend_from_slice(&chunk[..n]);
    }
    Some(hex(&md5(&buf)))
}

pub struct Account {
    pub server: String,
    pub user: String,
    pub key: String, // md5(password), as KOReader stores it
}

impl Account {
    pub fn from_config(c: &crate::sys::Config) -> Option<Account> {
        let user = c.get("sync_user").to_string();
        if user.is_empty() {
            return None;
        }
        let server = Some(c.get("sync_server").to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| DEFAULT_SERVER.into());
        Some(Account { server, user, key: c.get("sync_key").to_string() })
    }

    fn curl(&self, method: &str, path: &str, body: Option<&str>) -> Result<(u32, String), String> {
        let url = format!("{}{}", self.server.trim_end_matches('/'), path);
        let mut args = vec!["-sS".to_string(), "-m".into(), "20".into(), "-X".into(), method.into(),
                            "-H".into(), "Accept: application/vnd.koreader.v1+json".into(),
                            "-H".into(), format!("x-auth-user: {}", self.user), "-H".into(), format!("x-auth-key: {}", self.key),
                            "-w".into(), "\n%{http_code}".into()];
        if let Some(b) = body {
            args.extend(["-H".into(), "Content-Type: application/json".into(), "-d".into(), b.into()]);
        }
        args.push(url);
        let out = Command::new("curl").args(&args).stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = text.rsplit_once('\n').unwrap_or((&text, "0"));
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("curl: ").to_string());
        }
        Ok((code.trim().parse().unwrap_or(0), body.to_string()))
    }

    /// Check the credentials (200) — 401 means unknown user or wrong password.
    pub fn authorize(&self) -> Result<bool, String> {
        let (code, _) = self.curl("GET", "/users/auth", None)?;
        Ok(code == 200)
    }

    pub fn register(&self) -> Result<(), String> {
        let body = format!("{{\"username\":\"{}\",\"password\":\"{}\"}}", esc(&self.user), self.key);
        match self.curl("POST", "/users/create", Some(&body))? {
            (201, _) | (200, _) => Ok(()),
            (402, _) => Err("That username is already taken.".into()),
            (c, b) => Err(format!("Registration failed ({c}) {b}")),
        }
    }

    /// Fetch the last saved progress: (percentage 0..1, progress string, device name).
    pub fn pull(&self, doc: &str) -> Result<Option<(f64, String, String)>, String> {
        let (code, body) = self.curl("GET", &format!("/syncs/progress/{doc}"), None)?;
        if code != 200 {
            return Ok(None);
        }
        let j = crate::json::parse(&body).map_err(|e| e.to_string())?;
        let Some(p) = j.get("percentage").num() else { return Ok(None) };
        Ok(Some((p, j.get("progress").str().unwrap_or("").to_string(), j.get("device").str().unwrap_or("another device").to_string())))
    }

    pub fn push(&self, doc: &str, percentage: f64, chapter: usize, device_id: &str) -> Result<(), String> {
        let body = format!("{{\"document\":\"{doc}\",\"progress\":\"/body/DocFragment[{}]/body\",\"percentage\":{:.4},\
                            \"device\":\"Kindle Linux\",\"device_id\":\"{}\"}}", chapter + 1, percentage, esc(device_id));
        match self.curl("PUT", "/syncs/progress", Some(&body))? {
            (200, _) | (202, _) => Ok(()),
            (c, b) => Err(format!("sync failed ({c}) {b}")),
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Chapter index from a KOReader xpointer ("/body/DocFragment[3]/body/p[5]/text().0" -> 2).
pub fn chapter_from_xpointer(x: &str) -> Option<usize> {
    let i = x.find("DocFragment[")? + "DocFragment[".len();
    let n: usize = x[i..].split(']').next()?.parse().ok()?;
    n.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"The quick brown fox jumps over the lazy dog")), "9e107d9d372bb6826bd81d3542a419d6");
        assert_eq!(chapter_from_xpointer("/body/DocFragment[3]/body/p[5]/text().0"), Some(2));
    }
}
