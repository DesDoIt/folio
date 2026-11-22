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
        return Err("This book is DRM-protected; only the stock Kindle can open it.".into());
    }
    if compression == 17480 {
        return Err("This MOBI uses HUFF/CDIC compression, which isn't supported yet. Open it in KOReader.".into());
    }
    let (enc, extra_flags, version) = if h.len() >= 0xf4 && &h[16..20] == b"MOBI" {
        let hlen = be32(h, 20);
        let flags = if hlen >= 0xe4 { be16(h, 0xf2) } else { 0 };
        (be32(h, 28), flags, be32(h, 0x24))
    } else {
        (1252, 0, 0)
    };
    let mut raw: Vec<u8> = vec![];
    for i in 1..=text_records {
        let Some(r) = rec(base + i) else { break };
        let r = strip_trailing(r, extra_flags);
        match compression {
            1 => raw.extend_from_slice(r),
            2 => raw.extend(palmdoc(r)),
            c => return Err(format!("Unknown MOBI compression {c}")),
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
