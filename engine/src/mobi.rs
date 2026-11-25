//! DRM-free MOBI / AZW3 (KF8) reading: PalmDB container, PalmDOC (LZ77) decompression,
//! trailing-entry stripping, EXTH metadata. The text is HTML-like and goes through the same
//! converter as EPUB chapters. HUFF/CDIC-compressed and DRM-protected files are reported.

use crate::epub::{html_to_blocks, Block, BlockKind};

fn be16(b: &[u8], i: usize) -> usize {
    u16::from_be_bytes([b[i], b[i + 1]]) as usize
}
fn be32(b: &[u8], i: usize) -> usize {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize
}

/// PalmDOC LZ77 decompression.
pub fn palmdoc(data: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(data.len() * 2);
    let mut i = 0;
    while i < data.len() {
        let c = data[i];
        i += 1;
        match c {
            0 | 0x09..=0x7f => out.push(c),
            1..=8 => {
                let n = c as usize;
                out.extend_from_slice(&data[i..(i + n).min(data.len())]);
                i += n;
            }
            0xc0..=0xff => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
            _ => {
                if i >= data.len() {
                    break;
                }
                let pair = ((c as usize) << 8) | data[i] as usize;
                i += 1;
                let dist = (pair >> 3) & 0x7ff;
                let n = (pair & 7) + 3;
                if dist == 0 || dist > out.len() {
                    continue;
                }
                for _ in 0..n {
                    let b = out[out.len() - dist];
                    out.push(b);
                }
            }
        }
    }
    out
}

/// HUFF/CDIC decompression (the dictionary scheme some MOBI files use instead of PalmDOC).
pub struct HuffCdic {
    dict1: Vec<(u32, bool, u64)>, // (code length, terminal, max code) by the code's top byte
    mincode: Vec<u64>,
    maxcode: Vec<u64>,
    dict: Vec<(Vec<u8>, bool)>, // phrases; bool = already expanded
}

impl HuffCdic {
    pub fn new(recs: &[&[u8]]) -> Result<HuffCdic, String> {
        let bad = || "This MOBI's HUFF/CDIC dictionary is damaged.".to_string();
        let huff = recs.first().ok_or_else(bad)?;
        if huff.len() < 24 || &huff[0..4] != b"HUFF" {
            return Err(bad());
        }
        let (off1, off2) = (be32(huff, 8), be32(huff, 12));
        if huff.len() < off1 + 1024 || huff.len() < off2 + 256 {
            return Err(bad());
        }
        let mut dict1 = Vec::with_capacity(256);
        for i in 0..256 {
            let v = be32(huff, off1 + i * 4) as u64;
            let (len, term, max) = ((v & 0x1f) as u32, v & 0x80 != 0, v >> 8);
            if len == 0 {
                return Err(bad());
            }
            dict1.push((len, term, ((max + 1) << (32 - len)) - 1));
        }
        let (mut mincode, mut maxcode) = (vec![0u64], vec![0u64]);
        for len in 1..=32u32 {
            let i = (len as usize - 1) * 2;
            let (mn, mx) = (be32(huff, off2 + i * 4) as u64, be32(huff, off2 + (i + 1) * 4) as u64);
            mincode.push(mn << (32 - len));
            maxcode.push(((mx + 1) << (32 - len)) - 1);
        }
        let mut dict = vec![];
        for c in &recs[1..] {
            if c.len() < 16 || &c[0..4] != b"CDIC" {
                return Err(bad());
            }
            let (phrases, bits) = (be32(c, 8), be32(c, 12));
            let n = (1usize << bits).min(phrases.saturating_sub(dict.len()));
            for k in 0..n {
                let off = be16(c, 16 + k * 2);
                let p = 16 + off;
                if p + 2 > c.len() {
                    return Err(bad());
                }
                let blen = be16(c, p);
                let data = c.get(p + 2..(p + 2 + (blen & 0x7fff)).min(c.len())).unwrap_or(&[]).to_vec();
                dict.push((data, blen & 0x8000 != 0));
            }
        }
        Ok(HuffCdic { dict1, mincode, maxcode, dict })
    }

    pub fn unpack(&mut self, data: &[u8], depth: u32) -> Vec<u8> {
        let mut out = vec![];
        if depth > 32 {
            return out;
        }
        let mut bitsleft = data.len() as i64 * 8;
        let mut buf = data.to_vec();
        buf.extend_from_slice(&[0; 8]);
        let rd = |p: usize| u64::from_be_bytes(buf[p..p + 8].try_into().unwrap());
        let mut pos = 0;
        let mut x = rd(pos);
        let mut n: i64 = 32;
        loop {
            if n <= 0 {
                pos += 4;
                if pos + 8 > buf.len() {
                    break;
                }
                x = rd(pos);
                n += 32;
            }
            let code = (x >> n) & 0xffff_ffff;
            let (mut len, term, mut max) = self.dict1[(code >> 24) as usize];
            if !term {
                while len < 32 && code < self.mincode[len as usize] {
                    len += 1;
                }
                max = self.maxcode[len as usize];
            }
            n -= len as i64;
            bitsleft -= len as i64;
            if bitsleft < 0 {
                break;
            }
            let r = ((max - code) >> (32 - len)) as usize;
            let Some((slice, done)) = self.dict.get(r).cloned() else { break };
            let slice = if done {
                slice
            } else {
                let s = self.unpack(&slice, depth + 1);
                self.dict[r] = (s.clone(), true);
                s
            };
            out.extend_from_slice(&slice);
        }
        out
    }
}

/// Size of a backward-encoded trailing entry ending at `end`.
fn trailing_entry_size(data: &[u8], end: usize) -> usize {
    let (mut bitpos, mut result, mut size) = (0u32, 0usize, end);
    while size > 0 {
        let v = data[size - 1];
        result |= ((v & 0x7f) as usize) << bitpos;
        bitpos += 7;
        size -= 1;
        if v & 0x80 != 0 || bitpos >= 28 {
            break;
        }
    }
    result
}

/// Strip the per-record trailing entries described by `flags` (MOBI header extra_data_flags).
pub fn strip_trailing(data: &[u8], flags: usize) -> &[u8] {
    let mut end = data.len();
    let mut f = flags >> 1;
    while f != 0 && end > 0 {
        if f & 1 != 0 {
            let n = trailing_entry_size(data, end);
            end = end.saturating_sub(n);
        }
        f >>= 1;
    }
    if flags & 1 != 0 && end > 0 {
        let n = (data[end - 1] & 3) as usize + 1;
        end = end.saturating_sub(n);
    }
    &data[..end]
}

fn cp1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = ['€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
                              '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ'];
    bytes.iter().map(|&b| if (0x80..0xa0).contains(&b) { HIGH[(b - 0x80) as usize] } else { b as char }).collect()
}

pub struct MobiBook {
    pub title: String,
    pub author: String,
    pub chapters: Vec<(String, Vec<Block>)>,
}

/// Load a DRM-free MOBI/AZW3 file.
pub fn load(path: &str) -> Result<MobiBook, String> {
    let f = std::fs::read(path).map_err(|e| e.to_string())?;
    if f.len() < 86 || &f[60..68] != b"BOOKMOBI" && &f[60..68] != b"TEXtREAd" {
        return Err("Not a MOBI/AZW3 file".into());
    }
    let n = be16(&f, 76);
    let rec = |i: usize| -> Option<&[u8]> {
        if i >= n {
            return None;
        }
        let s = be32(&f, 78 + i * 8);
        let e = if i + 1 < n { be32(&f, 78 + (i + 1) * 8) } else { f.len() };
        f.get(s..e.min(f.len()).max(s))
    };
    // section start: KF8 part of a combined file if present (EXTH 121), else record 0
    let r0 = rec(0).ok_or("missing header record")?;
    let (mut title, mut author, mut kf8_boundary) = (String::new(), String::new(), None);
    let parse_exth = |r0: &[u8], title: &mut String, author: &mut String, boundary: &mut Option<usize>| {
        if r0.len() < 0x84 || &r0[16..20] != b"MOBI" {
            return;
        }
        let hlen = be32(r0, 20);
        let enc = be32(r0, 28);
        let decode = |b: &[u8]| if enc == 1252 { cp1252(b) } else { String::from_utf8_lossy(b).into_owned() };
        let (no, nl) = (be32(r0, 0x54), be32(r0, 0x58));
        if title.is_empty() && no + nl <= r0.len() && nl > 0 {
            *title = decode(&r0[no..no + nl]);
        }
        if be32(r0, 0x80) & 0x40 != 0 {
            let x = 16 + hlen;
            if r0.len() >= x + 12 && &r0[x..x + 4] == b"EXTH" {
                let count = be32(r0, x + 8);
                let mut p = x + 12;
                for _ in 0..count {
                    if p + 8 > r0.len() {
                        break;
                    }
                    let (t, l) = (be32(r0, p), be32(r0, p + 4));
                    if l < 8 || p + l > r0.len() {
                        break;
                    }
                    let d = &r0[p + 8..p + l];
                    match t {
                        100 if author.is_empty() => *author = decode(d),
                        503 => *title = decode(d),
                        121 if d.len() >= 4 => *boundary = Some(be32(d, 0)),
                        _ => {}
                    }
                    p += l;
                }
            }
        }
    };
    parse_exth(r0, &mut title, &mut author, &mut kf8_boundary);
    let base = match kf8_boundary {
        Some(b) if b != 0xffff_ffff && rec(b).map_or(false, |r| r.len() > 20 && &r[16..20] == b"MOBI") => b,
        _ => 0,
    };
    let h = rec(base).ok_or("missing header record")?;
    let compression = be16(h, 0);
    let text_records = be16(h, 8);
    if be16(h, 12) != 0 {
        return Err("This book is DRM-protected, so it only opens in the app or device it was bought for.".into());
    }
    // HUFF/CDIC (dictionary) compression: records listed in the MOBI header at 0x70/0x74
    let huff = if compression == 17480 {
        let (off, num) = (be32(h, 0x70), be32(h, 0x74));
        let recs: Vec<&[u8]> = (0..num).filter_map(|i| rec(base + off + i).or_else(|| rec(off + i))).collect();
        Some(HuffCdic::new(&recs)?)
    } else {
        None
    };
    let (enc, extra_flags, version) = if h.len() >= 0xf4 && &h[16..20] == b"MOBI" {
        let hlen = be32(h, 20);
        let flags = if hlen >= 0xe4 { be16(h, 0xf2) } else { 0 };
        (be32(h, 28), flags, be32(h, 0x24))
    } else {
        (1252, 0, 0)
    };
    let mut huff = huff;
    let mut raw: Vec<u8> = vec![];
    for i in 1..=text_records {
        let Some(r) = rec(base + i) else { break };
        let r = strip_trailing(r, extra_flags);
        match (compression, huff.as_mut()) {
            (1, _) => raw.extend_from_slice(r),
            (2, _) => raw.extend(palmdoc(r)),
            (17480, Some(hc)) => raw.extend(hc.unpack(r, 0)),
            (c, _) => return Err(format!("Unknown MOBI compression {c}")),
        }
    }
    let text = if enc == 1252 { cp1252(&raw) } else { String::from_utf8_lossy(&raw).into_owned() };
    // split into chapters: page breaks (MOBI 6) or concatenated XHTML parts (KF8)
    let parts: Vec<String> = if version >= 8 || base > 0 {
        let mut v = vec![];
        let mut rest = text.as_str();
        while let Some(i) = rest[1.min(rest.len())..].find("<html").map(|i| i + 1) {
            v.push(rest[..i].to_string());
            rest = &rest[i..];
        }
        v.push(rest.to_string());
        v
    } else {
        text.split("<mbp:pagebreak").map(|p| format!("<{p}")).collect()
    };
    let mut chapters: Vec<(String, Vec<Block>)> = vec![];
    for p in parts {
        let mut blocks = html_to_blocks(&p);
        blocks.retain(|b| b.kind != BlockKind::Image); // MOBI images aren't supported yet
        if blocks.iter().all(|b| b.runs.iter().all(|r| r.text.trim().is_empty())) {
            continue;
        }
        let heading = blocks.iter().find(|b| matches!(b.kind, BlockKind::Heading(_)))
            .map(|b| b.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .filter(|t| !t.trim().is_empty() && t.chars().count() < 80);
        let name = heading.unwrap_or_else(|| format!("Part {}", chapters.len() + 1));
        let mut off = 0;
        for b in blocks.iter_mut() {
            b.start = off;
            off += b.chars();
        }
        chapters.push((name, blocks));
    }
    if chapters.is_empty() {
        return Err("No readable text found in this file.".into());
    }
    if title.is_empty() {
        title = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    Ok(MobiBook { title, author, chapters })
}

/// The cover picture of a MOBI/AZW3 file (EXTH 201 = offset from the first image record), if any.
pub fn cover(path: &str) -> Option<Vec<u8>> {
    let f = std::fs::read(path).ok()?;
    if f.len() < 86 || &f[60..68] != b"BOOKMOBI" {
        return None;
    }
    let n = be16(&f, 76);
    let rec = |i: usize| -> Option<&[u8]> {
        if i >= n {
            return None;
        }
        let s = be32(&f, 78 + i * 8);
        let e = if i + 1 < n { be32(&f, 78 + (i + 1) * 8) } else { f.len() };
        f.get(s..e.min(f.len()).max(s))
    };
    let r0 = rec(0)?;
    if r0.len() < 0x84 || &r0[16..20] != b"MOBI" {
        return None;
    }
    let first_img = be32(r0, 0x6c);
    if first_img == 0xffff_ffff || first_img == 0 {
        return None;
    }
    let mut off = None;
    if be32(r0, 0x80) & 0x40 != 0 {
        let x = 16 + be32(r0, 20);
        if r0.len() >= x + 12 && &r0[x..x + 4] == b"EXTH" {
            let mut p = x + 12;
            for _ in 0..be32(r0, x + 8) {
                if p + 8 > r0.len() {
                    break;
                }
                let (t, l) = (be32(r0, p), be32(r0, p + 4));
                if l < 8 || p + l > r0.len() {
                    break;
                }
                if t == 201 && l >= 12 {
                    off = Some(be32(r0, p + 8));
                }
                p += l;
            }
        }
    }
    let img = rec(first_img + off.unwrap_or(0))?;
    let is_img = img.starts_with(&[0xff, 0xd8]) || img.starts_with(b"\x89PNG") || img.starts_with(b"GIF8");
    is_img.then(|| img.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palmdoc_decompresses() {
        // literal "abc", then a back-reference of length 3 at distance 3 => "abcabc"; 0xC1 => " A"
        let pair: u16 = (0x80 << 8) | (3 << 3) | 0;
        let data = [b'a', b'b', b'c', (pair >> 8) as u8, pair as u8, 0xC1];
        assert_eq!(palmdoc(&data), b"abcabc A");
    }

    #[test]
    fn trailing_entries() {
        // one trailing entry of size 3 (backward varint 0x83 = 3 with stop bit) + multibyte flag (1 extra byte)
        let rec = [b'x', b'y', 0x00, b'p', b'q', 0x83];
        assert_eq!(strip_trailing(&rec, 0b10), b"xy\x00p".get(..3).unwrap()); // the size includes the varint byte
        let rec2 = [b'x', b'y', b'z', 0x00];
        assert_eq!(strip_trailing(&rec2, 0b1), b"xyz");
    }
}
