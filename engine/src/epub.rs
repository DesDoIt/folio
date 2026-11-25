//! Book loading for the built-in reader: a tiny zip reader, EPUB (OPF spine + TOC), plain text,
//! and a forgiving XHTML -> text-blocks converter. No external parsers.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

// --- Zip --------------------------------------------------------------------------------
struct ZipEntry {
    method: u16,
    comp_size: u32,
    /// uncompressed size
    size: u32,
    local_offset: u32,
}

pub struct Zip {
    file: File,
    entries: HashMap<String, ZipEntry>,
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

impl Zip {
    pub fn open(path: &str) -> Result<Zip, String> {
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let len = file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        let tail_len = len.min(66_000);
        file.seek(SeekFrom::Start(len - tail_len)).map_err(|e| e.to_string())?;
        let mut tail = vec![0u8; tail_len as usize];
        file.read_exact(&mut tail).map_err(|e| e.to_string())?;
        let eocd = (0..tail.len().saturating_sub(21)).rev()
            .find(|&i| u32le(&tail, i) == 0x0605_4b50)
            .ok_or("not a zip file (no end of central directory)")?;
        let count = u16le(&tail, eocd + 10) as usize;
        let cd_size = u32le(&tail, eocd + 12) as usize;
        let cd_off = u32le(&tail, eocd + 16) as u64;
        file.seek(SeekFrom::Start(cd_off)).map_err(|e| e.to_string())?;
        let mut cd = vec![0u8; cd_size];
        file.read_exact(&mut cd).map_err(|e| e.to_string())?;
        let mut entries = HashMap::new();
        let mut i = 0;
        for _ in 0..count {
            if i + 46 > cd.len() || u32le(&cd, i) != 0x0201_4b50 {
                break;
            }
            let method = u16le(&cd, i + 10);
            let comp_size = u32le(&cd, i + 20);
            let size = u32le(&cd, i + 24);
            let name_len = u16le(&cd, i + 28) as usize;
            let extra_len = u16le(&cd, i + 30) as usize;
            let comment_len = u16le(&cd, i + 32) as usize;
            let local_offset = u32le(&cd, i + 42);
            let name = String::from_utf8_lossy(&cd[i + 46..i + 46 + name_len]).into_owned();
            entries.insert(name, ZipEntry { method, comp_size, size, local_offset });
            i += 46 + name_len + extra_len + comment_len;
        }
        Ok(Zip { file, entries })
    }

    pub fn read(&mut self, name: &str) -> Option<Vec<u8>> {
        let e = self.entries.get(name).or_else(|| {
            // tolerate case differences in hrefs
            let lower = name.to_lowercase();
            self.entries.iter().find(|(k, _)| k.to_lowercase() == lower).map(|(_, v)| v)
        })?;
        let (method, comp_size, off) = (e.method, e.comp_size as usize, e.local_offset as u64);
        let mut hdr = [0u8; 30];
        self.file.seek(SeekFrom::Start(off)).ok()?;
        self.file.read_exact(&mut hdr).ok()?;
        if u32le(&hdr, 0) != 0x0403_4b50 {
            return None;
        }
        let skip = u16le(&hdr, 26) as u64 + u16le(&hdr, 28) as u64;
        self.file.seek(SeekFrom::Start(off + 30 + skip)).ok()?;
        let mut data = vec![0u8; comp_size];
        self.file.read_exact(&mut data).ok()?;
        match method {
            0 => Some(data),
            8 => miniz_oxide::inflate::decompress_to_vec(&data).ok(),
            _ => None,
        }
    }

    /// Names of all entries.
    /// Uncompressed size of an entry.
    pub fn size(&self, name: &str) -> Option<u32> {
        self.entries.get(name).map(|e| e.size)
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn read_string(&mut self, name: &str) -> Option<String> {
        self.read(name).map(|b| String::from_utf8_lossy(&b).into_owned())
    }
}

// --- Minimal XML/HTML tokenizer --------------------------------------------------------------
pub enum Tok<'a> {
    Open(String, HashMap<String, String>, bool), // name (lowercase, no namespace), attrs, self-closing
    Close(String),
    Text(&'a str),
}

fn local_name(n: &str) -> String {
    n.rsplit(':').next().unwrap_or(n).to_ascii_lowercase()
}

fn parse_attrs(s: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let ks = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() && b[i] != b'/' {
            i += 1;
        }
        let key = &s[ks..i];
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                m.insert(local_name(key), decode_entities(&s[vs..i.min(b.len())]));
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                m.insert(local_name(key), decode_entities(&s[vs..i]));
            }
        } else if !key.is_empty() {
            m.insert(local_name(key), String::new());
        } else {
            i += 1;
        }
    }
    m
}

pub fn tokenize(src: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            if src[i..].starts_with("<!--") {
                i = src[i..].find("-->").map_or(b.len(), |e| i + e + 3);
                continue;
            }
            if src[i..].starts_with("<![CDATA[") {
                let end = src[i..].find("]]>").map_or(b.len(), |e| i + e);
                out.push(Tok::Text(&src[i + 9..end]));
                i = (end + 3).min(b.len());
                continue;
            }
            // find the closing '>' of this tag, skipping any '>' inside quoted attribute values
            // (Wikipedia stores JSON with "</ref>" etc. in data-* attributes)
            let end = {
                let mut j = i + 1;
                let mut quote = 0u8;
                while j < b.len() {
                    let c = b[j];
                    if quote != 0 {
                        if c == quote {
                            quote = 0;
                        }
                    } else if c == b'"' || c == b'\'' {
                        // only treat as a quote when it starts an attribute value (after '=')
                        let prev = src[..j].trim_end().as_bytes().last().copied();
                        if prev == Some(b'=') {
                            quote = c;
                        }
                    } else if c == b'>' {
                        break;
                    }
                    j += 1;
                }
                if j >= b.len() {
                    break;
                }
                j
            };
            let inner = &src[i + 1..end];
            i = end + 1;
            if inner.starts_with('!') || inner.starts_with('?') {
                continue;
            }
            if let Some(name) = inner.strip_prefix('/') {
                out.push(Tok::Close(local_name(name.trim())));
                continue;
            }
            let self_closing = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let (name, rest) = match inner.find(|c: char| c.is_whitespace()) {
                Some(p) => (&inner[..p], &inner[p..]),
                None => (inner, ""),
            };
            let lname = local_name(name);
            let raw = !self_closing && (lname == "script" || lname == "style");
            out.push(Tok::Open(lname.clone(), parse_attrs(rest), self_closing));
            if raw {
                // script/style contents are plain text: jump to the real closing tag (a "<script" or
                // "</div>" inside JavaScript strings must not count as markup)
                let close = format!("</{lname}");
                let mut j = i;
                let end = loop {
                    match src[j..].find("</") {
                        Some(k) => {
                            let at = j + k;
                            if src[at..].len() >= close.len() && src[at..at + close.len()].eq_ignore_ascii_case(&close) {
                                break at;
                            }
                            j = at + 2;
                        }
                        None => break b.len(),
                    }
                };
                i = end;
            }
        } else {
            let end = src[i..].find('<').map_or(b.len(), |e| i + e);
            out.push(Tok::Text(&src[i..end]));
            i = end;
        }
    }
    out
}

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let mut lim = rest.len().min(12);
        while !rest.is_char_boundary(lim) {
            lim -= 1;
        }
        let Some(semi) = rest[..lim].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "shy" => Some('\u{ad}'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "copy" => Some('©'),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// --- Text blocks ------------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BlockKind {
    Para,
    Heading(u8),
    Pre,
    Item,
    Quote,
    Rule,
    /// An image; its source path is in `runs[0].text` (resolved to a zip path by `Book::chapter_blocks`).
    Image,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    /// Index into the document's link list (web pages), if this text is a link.
    pub link: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub kind: BlockKind,
    pub runs: Vec<Run>,
    /// character offset of this block within its chapter (for saving positions)
    pub start: usize,
}

impl Block {
    pub fn chars(&self) -> usize {
        self.runs.iter().map(|r| r.text.chars().count()).sum::<usize>() + 1
    }
}

struct Builder {
    blocks: Vec<Block>,
    cur: Option<Block>,
    offset: usize,
    bold: u32,
    italic: u32,
    pre: u32,
    quote: u32,
    space_pending: bool,
    link: Option<u32>,
    links: Vec<String>,
    /// element ids seen since the last flushed block (assigned to the next block's start)
    pending_ids: Vec<String>,
    anchors: HashMap<String, usize>,
}

impl Builder {
    fn new() -> Builder {
        Builder { blocks: vec![], cur: None, offset: 0, bold: 0, italic: 0, pre: 0, quote: 0, space_pending: false, link: None,
                  links: vec![], pending_ids: vec![], anchors: HashMap::new() }
    }

    fn flush(&mut self) {
        if let Some(mut b) = self.cur.take() {
            if let Some(last) = b.runs.last_mut() {
                let t = last.text.trim_end().to_string();
                last.text = t;
            }
            b.runs.retain(|r| !r.text.is_empty());
            if !b.runs.is_empty() || b.kind == BlockKind::Rule {
                b.start = self.offset;
                for id in self.pending_ids.drain(..) {
                    self.anchors.insert(id, b.start);
                }
                self.offset += b.chars();
                self.blocks.push(b);
            }
        }
        self.space_pending = false;
    }

    fn start(&mut self, kind: BlockKind) {
        self.flush();
        self.cur = Some(Block { kind, runs: vec![], start: 0 });
    }

    fn text(&mut self, t: &str) {
        // soft hyphens (U+00AD) only mark possible breaks; our own hyphenation handles breaking
        let owned;
        let t = if t.contains('\u{ad}') {
            owned = t.replace('\u{ad}', "");
            owned.as_str()
        } else {
            t
        };
        let pre = self.pre > 0;
        let blk = self.cur.get_or_insert_with(|| Block { kind: BlockKind::Para, runs: vec![], start: 0 });
        let bold = self.bold > 0 || matches!(blk.kind, BlockKind::Heading(_));
        let italic = self.italic > 0;
        let mut s = String::new();
        if pre {
            s.push_str(t);
        } else {
            for (i, word) in t.split(|c: char| c.is_whitespace() && c != '\u{a0}').enumerate() {
                if i > 0 {
                    self.space_pending = true;
                }
                if word.is_empty() {
                    continue;
                }
                let empty = s.is_empty() && blk.runs.iter().all(|r| r.text.is_empty());
                if self.space_pending && !empty {
                    s.push(' ');
                }
                self.space_pending = false;
                s.push_str(word);
            }
            if t.ends_with(|c: char| c.is_whitespace() && c != '\u{a0}') {
                self.space_pending = true;
            }
        }
        if s.is_empty() {
            return;
        }
        match blk.runs.last_mut() {
            Some(r) if r.bold == bold && r.italic == italic && r.link == self.link => r.text.push_str(&s),
            _ => blk.runs.push(Run { text: s, bold, italic, link: self.link }),
        }
    }
}

/// Convert an XHTML chapter into text blocks.
pub fn html_to_blocks(html: &str) -> Vec<Block> {
    html_to_doc(html, false).0
}

/// A chapter with its link targets and element-id anchors (id -> character offset).
pub struct Doc {
    pub blocks: Vec<Block>,
    pub links: Vec<String>,
    pub anchors: HashMap<String, usize>,
}

pub fn html_to_full(html: &str) -> Doc {
    let (blocks, links, anchors) = html_to_doc_inner(html, false);
    Doc { blocks, links, anchors }
}

/// HTML -> (blocks, link targets). Links become runs with `link: Some(index)`.
/// `reader_view` keeps only <article>/<main> when present and drops nav/header/footer/aside.
pub fn html_to_doc(html: &str, reader_view: bool) -> (Vec<Block>, Vec<String>) {
    let (blocks, links, _) = html_to_doc_inner(html, reader_view);
    if reader_view {
        // Some news sites only put the article in their search-engine metadata (JSON-LD articleBody)
        // and fill the page with scripts. Use it when it has clearly more text than the page.
        let shown: usize = blocks.iter().map(|b| b.chars()).sum();
        if let Some(body) = jsonld_article_body(html) {
            let n = body.chars().count();
            if n >= 400 && n as f32 > shown as f32 * 1.3 {
                return (article_text_blocks(&body), vec![]);
            }
        }
        let blocks = drop_share_lists(blocks);
        // break up walls of text (plain paragraphs only, so links and formatting stay intact)
        let mut out = Vec::with_capacity(blocks.len());
        let mut off = 0;
        for b in blocks {
            let plain = b.kind == BlockKind::Para && b.runs.len() == 1 && b.runs[0].link.is_none();
            let pieces = if plain { split_long_paragraph(b.runs[0].text.clone()) } else { vec![] };
            if pieces.len() > 1 {
                for p in pieces {
                    let nb = Block { kind: BlockKind::Para, runs: vec![Run { text: p, ..b.runs[0].clone() }], start: off };
                    off += nb.chars();
                    out.push(nb);
                }
            } else {
                let mut b = b;
                b.start = off;
                off += b.chars();
                out.push(b);
            }
        }
        return (out, links);
    }
    (blocks, links)
}

/// Labels of "share this" buttons, which reading view leaves out.
const SHARE_LABELS: &[&str] = &["copy link", "copy", "email", "e-mail", "mail", "print", "share", "facebook", "twitter",
    "x", "telegram", "linkedin", "whatsapp", "reddit", "pinterest", "tumblr", "pocket", "flipboard", "threads", "bluesky",
    "mastodon", "koo", "sms", "messenger", "share on facebook", "share on twitter", "share on x", "share on whatsapp"];

/// Drop runs of three or more list items that are all share-button labels.
fn drop_share_lists(blocks: Vec<Block>) -> Vec<Block> {
    let is_share = |b: &Block| {
        b.kind == BlockKind::Item && {
            let t: String = b.runs.iter().map(|r| r.text.as_str()).collect::<String>().trim().to_lowercase();
            SHARE_LABELS.contains(&t.as_str())
        }
    };
    let mut out = Vec::with_capacity(blocks.len());
    let mut i = 0;
    while i < blocks.len() {
        let mut j = i;
        while j < blocks.len() && is_share(&blocks[j]) {
            j += 1;
        }
        if j - i >= 3 {
            i = j;
            continue;
        }
        out.push(blocks[i].clone());
        i += 1;
    }
    out
}

/// The longest JSON-LD "articleBody" string in a page, decoded (scanned by hand: these JSON blobs
/// are often not strictly valid).
pub fn jsonld_article_body(html: &str) -> Option<String> {
    let mut best: Option<String> = None;
    let mut rest = html;
    while let Some(k) = rest.find("\"articleBody\"") {
        rest = &rest[k + 13..];
        let t = rest.trim_start();
        let Some(t) = t.strip_prefix(':') else { continue };
        let t = t.trim_start();
        let Some(t) = t.strip_prefix('"') else { continue };
        let mut out = String::new();
        let mut chars = t.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') | Some('r') => out.push(' '),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            out.push(ch);
                        }
                    }
                    Some(o) => out.push(o),
                    None => break,
                },
                c => out.push(c),
            }
        }
        if best.as_ref().map_or(true, |b| out.len() > b.len()) {
            best = Some(out);
        }
    }
    let body = decode_entities(&best?);
    // it may itself be HTML
    let body = if body.contains("<p") || body.contains("<br") {
        html_to_doc(&body, false).0.iter().map(|b| b.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .collect::<Vec<_>>().join("\n")
    } else {
        body
    };
    Some(strip_css(body.trim()))
}

/// Split a very long paragraph at sentence ends (". " + capital) into pieces of ~400–700 characters.
fn split_long_paragraph(p: String) -> Vec<String> {
    if p.chars().count() <= 900 {
        return vec![p];
    }
    const ABBR: &[&str] = &["Mr", "Mrs", "Ms", "Dr", "No", "Nos", "vs", "v", "Ltd", "Co", "St", "Sr", "Jr", "Hon", "Rs", "Art", "Sec", "Ors", "Anr", "viz", "i.e", "e.g"];
    let c: Vec<char> = p.chars().collect();
    let mut out = vec![];
    let mut start = 0;
    let mut i = 0;
    while i + 2 < c.len() {
        let len = i.saturating_sub(start);
        if len >= 400 && matches!(c[i], '.' | '?' | '!') && c[i + 1] == ' ' && (c[i + 2].is_uppercase() || c[i + 2] == '“' || c[i + 2] == '"') {
            let word: String = c[start..i].iter().rev().take_while(|ch| !ch.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
            let abbr = ABBR.iter().any(|a| word.eq_ignore_ascii_case(a)) || (word.len() == 1 && word.chars().all(char::is_uppercase));
            if !abbr || len > 900 {
                out.push(c[start..=i].iter().collect::<String>().trim().to_string());
                start = i + 2;
                i = start;
                continue;
            }
        }
        i += 1;
    }
    out.push(c[start..].iter().collect::<String>().trim().to_string());
    out.retain(|s| !s.is_empty());
    out
}

/// Remove CSS that leaked into text (".ad { display: none; } @media (max-width: 767px) { … }"),
/// as some sites' JSON-LD article bodies contain. A rule is a selector starting with ".", "#" or "@"
/// at a word start, then a balanced { … } block containing a "property: value".
pub fn strip_css(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let sel_char = |ch: char| ch.is_alphanumeric() || " _-.#:,()>+~*[]=\"'%".contains(ch);
    while i < c.len() {
        let starts_rule = matches!(c[i], '.' | '#' | '@')
            && (i == 0 || c[i - 1].is_whitespace())
            && c.get(i + 1).map_or(false, |n| n.is_alphabetic() || *n == '-' || *n == '_');
        if starts_rule {
            // selector up to "{", within a short distance and only selector-ish characters
            let mut j = i + 1;
            while j < c.len() && j - i < 160 && c[j] != '{' && sel_char(c[j]) {
                j += 1;
            }
            if j < c.len() && c[j] == '{' {
                let mut depth = 0;
                let mut k = j;
                while k < c.len() {
                    match c[k] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                let body: String = c[j..k.min(c.len())].iter().collect();
                if k < c.len() && body.contains(':') {
                    i = k + 1;
                    // a following rule often starts right away; collapse the gap
                    while i < c.len() && c[i].is_whitespace() && out.ends_with(char::is_whitespace) {
                        i += 1;
                    }
                    if !out.ends_with(char::is_whitespace) {
                        out.push(' ');
                    }
                    continue;
                }
            }
        }
        out.push(c[i]);
        i += 1;
    }
    out.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
        .replace(" \n", "\n").replace("\n ", "\n")
}

/// Plain text to paragraphs: on line breaks, or (when the text has none) where one sentence runs
/// straight into the next without a space ("…assistance.“At the age…"), as JSON-LD bodies often do.
fn article_text_blocks(text: &str) -> Vec<Block> {
    let mut paras: Vec<String> = vec![];
    if text.contains('\n') {
        paras = text.split('\n').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    } else {
        let chars: Vec<char> = text.chars().collect();
        let mut cur = String::new();
        for (i, &c) in chars.iter().enumerate() {
            cur.push(c);
            let next = chars.get(i + 1).copied();
            let ends = matches!(c, '.' | '?' | '!' | '”' | '"' | ')');
            if ends && next.map_or(false, |n| n.is_uppercase() || n == '“') && cur.len() > 40 {
                // "U.S." / "Mr.X" style abbreviations: only split when the previous char isn't uppercase
                let prev = if i > 0 { chars[i - 1] } else { ' ' };
                if !(c == '.' && prev.is_uppercase()) {
                    paras.push(std::mem::take(&mut cur).trim().to_string());
                }
            }
        }
        if !cur.trim().is_empty() {
            paras.push(cur.trim().to_string());
        }
    }
    // walls of text: break paragraphs longer than ~900 characters every few sentences
    let paras: Vec<String> = paras.into_iter().flat_map(split_long_paragraph).collect();
    let mut off = 0;
    paras.into_iter().map(|p| {
        let b = Block { kind: BlockKind::Para, runs: vec![Run { text: p, bold: false, italic: false, link: None }], start: off };
        off += b.chars();
        b
    }).collect()
}

/// Reading view: find the element holding the article, Readability-style. Every paragraph scores
/// for its parent (and half for its grandparent) by length and commas; class/id names like "article"
/// or "story-body" boost a candidate, names like "comment", "subscribe" or "sidebar" sink it.
/// Returns the token range (open..=close) of the best element.
pub fn main_content(toks: &[Tok]) -> Option<(usize, usize)> {
    const VOID: &[&str] = &["img", "br", "hr", "meta", "link", "input", "source", "wbr", "area", "col", "base", "embed", "param", "track"];
    const SKIP: &[&str] = &["script", "style", "noscript", "svg", "template", "head", "nav", "header", "footer", "aside", "form", "button", "select"];
    struct El { name: String, start: usize, score: f32, weight: f32 }
    let weight_of = |attrs: &HashMap<String, String>, name: &str| -> f32 {
        let key = format!("{} {} {}", attrs.get("class").map_or("", |s| s.as_str()), attrs.get("id").map_or("", |s| s.as_str()),
                          attrs.get("itemprop").map_or("", |s| s.as_str())).to_ascii_lowercase();
        let mut w = 1.0;
        if key.contains("articlebody") || key.contains("article-body") || key.contains("story-body") || key.contains("entry-content")
            || key.contains("post-content") || key.contains("article-content") || key.contains("story-content") {
            w *= 2.5;
        } else if ["article", "content", "story", "entry", "post", "text", "body", "main"].iter().any(|k| key.contains(k)) {
            w *= 1.4;
        }
        if ["comment", "subscri", "login", "account", "footer", "header", "menu", "sidebar", "related", "promo", "share",
            "social", "newsletter", "advert", "banner", "popup", "modal", "paywall", "cookie", "trending", "recommend", "widget"]
            .iter().any(|k| key.contains(k)) {
            w *= 0.15;
        }
        if name == "article" || name == "main" {
            w *= 1.3;
        }
        w
    };
    let mut stack: Vec<El> = vec![];
    let mut skip: Vec<String> = vec![]; // open skipped elements; their contents (scripts!) are ignored
    let mut para: Option<(usize, usize, usize)> = None; // (stack depth of <p>, chars, commas)
    let mut best: Option<(f32, usize, usize)> = None;
    for (i, t) in toks.iter().enumerate() {
        if let Some(top) = skip.last() {
            match t {
                Tok::Close(n) if n == top => {
                    skip.pop();
                }
                Tok::Open(n, _, false) if n == top => skip.push(n.clone()),
                _ => {}
            }
            continue;
        }
        match t {
            Tok::Open(n, a, sc) => {
                if SKIP.contains(&n.as_str()) && !sc {
                    skip.push(n.clone());
                    continue;
                }
                if *sc || VOID.contains(&n.as_str()) {
                    continue;
                }
                if n == "p" {
                    para = Some((stack.len(), 0, 0));
                }
                let w = weight_of(a, n);
                stack.push(El { name: n.clone(), start: i, score: 0.0, weight: w });
            }
            Tok::Text(x) => {
                if let Some(p) = para.as_mut() {
                    p.1 += x.trim().chars().count();
                    p.2 += x.matches(',').count();
                } else {
                    // long text straight inside a container (sites that use <div> + <br> instead of <p>)
                    let len = x.trim().chars().count();
                    if len >= 80 {
                        if let Some(top) = stack.last_mut() {
                            top.score += 1.0 + (len as f32 / 100.0).min(3.0) + x.matches(',').count() as f32;
                        }
                    }
                }
            }
            Tok::Close(n) => {
                let Some(pos) = stack.iter().rposition(|e| &e.name == n) else { continue };
                while stack.len() > pos {
                    let e = stack.pop().unwrap();
                    if e.name == "p" {
                        if let Some((d, len, commas)) = para.take().filter(|p| p.0 == stack.len()) {
                            let _ = d;
                            if len >= 25 {
                                let s = 1.0 + commas as f32 + (len as f32 / 100.0).min(3.0);
                                if let Some(parent) = stack.last_mut() {
                                    parent.score += s;
                                }
                                if stack.len() >= 2 {
                                    let gp = stack.len() - 2;
                                    stack[gp].score += s / 2.0;
                                }
                            }
                        }
                        continue;
                    }
                    let total = e.score * e.weight;
                    if total > best.map_or(0.0, |b| b.0) {
                        best = Some((total, e.start, i));
                    }
                }
            }
        }
    }
    best.filter(|b| b.0 >= 8.0).map(|b| (b.1, b.2))
}

fn html_to_doc_inner(html: &str, reader_view: bool) -> (Vec<Block>, Vec<String>, HashMap<String, usize>) {
    let mut b = Builder::new();
    let mut skip = 0u32; // inside head/script/style
    let body_start = html.find("<body").unwrap_or(0);
    let src = &html[body_start..];
    let junk: &[&str] = if reader_view { &["head", "script", "style", "title", "nav", "header", "footer", "aside", "form", "noscript", "svg", "button", "select"] }
                        else { &["head", "script", "style", "title", "noscript", "svg", "select"] };
    let mut toks = tokenize(src);
    if reader_view {
        // keep only the element that holds the article text
        if let Some((a, z)) = main_content(&toks) {
            toks.truncate(z + 1);
            toks.drain(..a);
        }
    }
    for tok in toks {
        match tok {
            Tok::Open(name, attrs, sc) => {
                if let Some(id) = attrs.get("id").or_else(|| if name == "a" { attrs.get("name") } else { None }) {
                    b.pending_ids.push(id.clone());
                }
                if junk.contains(&name.as_str()) && !sc {
                    skip += 1;
                    continue;
                }
                if skip > 0 {
                    continue;
                }
                match name.as_str() {
                    "p" | "div" | "section" | "article" | "dd" | "dt" | "tr" | "figcaption" | "caption" => {
                        // a <p> right inside a fresh <li> belongs to that item
                        let fresh_item = b.cur.as_ref().map_or(false, |c| c.kind == BlockKind::Item && c.runs.is_empty());
                        if !fresh_item {
                            b.start(if b.quote > 0 { BlockKind::Quote } else { BlockKind::Para });
                        }
                    }
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => b.start(BlockKind::Heading(name.as_bytes()[1] - b'0')),
                    "li" => b.start(BlockKind::Item),
                    "blockquote" => {
                        b.quote += 1;
                        b.start(BlockKind::Quote);
                    }
                    "pre" => {
                        b.start(BlockKind::Pre);
                        b.pre += 1;
                    }
                    "br" => {
                        let kind = b.cur.as_ref().map_or(BlockKind::Para, |c| c.kind);
                        b.flush();
                        b.cur = Some(Block { kind, runs: vec![], start: 0 });
                    }
                    "hr" => {
                        b.start(BlockKind::Rule);
                        b.flush();
                    }
                    "b" | "strong" if !sc => b.bold += 1,
                    "i" | "em" | "cite" | "var" | "dfn" if !sc => b.italic += 1,
                    "a" if !sc => {
                        if let Some(h) = attrs.get("href").filter(|h| !h.starts_with('#') && !h.starts_with("javascript:")) {
                            b.links.push(h.clone());
                            b.link = Some(b.links.len() as u32 - 1);
                        }
                    }
                    "td" | "th" => b.text(" "),
                    "img" | "image" => {
                        if let Some(src) = attrs.get("src").or_else(|| attrs.get("href")).filter(|s| !s.is_empty()) {
                            let kind = b.cur.as_ref().map(|c| c.kind);
                            b.flush();
                            b.cur = Some(Block { kind: BlockKind::Image,
                                                 runs: vec![Run { text: src.clone(), bold: false, italic: false, link: None }], start: 0 });
                            b.flush();
                            if let Some(k) = kind.filter(|k| *k != BlockKind::Image) {
                                b.cur = Some(Block { kind: k, runs: vec![], start: 0 });
                            }
                        } else if let Some(alt) = attrs.get("alt").filter(|a| !a.trim().is_empty()) {
                            b.text(&format!(" [{}] ", alt.trim()));
                        }
                    }
                    _ => {}
                }
            }
            Tok::Close(name) => {
                if junk.contains(&name.as_str()) {
                    skip = skip.saturating_sub(1);
                    continue;
                }
                if skip > 0 {
                    continue;
                }
                match name.as_str() {
                    "blockquote" => {
                        b.quote = b.quote.saturating_sub(1);
                        b.flush();
                    }
                    "p" | "div" | "section" | "article" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li"
                    | "dd" | "dt" | "tr" | "figcaption" | "caption" => b.flush(),
                    "pre" => {
                        b.pre = b.pre.saturating_sub(1);
                        b.flush();
                    }
                    "b" | "strong" => b.bold = b.bold.saturating_sub(1),
                    "i" | "em" | "cite" | "var" | "dfn" => b.italic = b.italic.saturating_sub(1),
                    "a" => b.link = None,
                    _ => {}
                }
            }
            Tok::Text(t) => {
                if skip == 0 {
                    let d = decode_entities(t);
                    b.text(&d);
                }
            }
        }
    }
    b.flush();
    let mut anchors = b.anchors;
    for id in b.pending_ids {
        anchors.insert(id, b.offset);
    }
    (b.blocks, b.links, anchors)
}

/// Plain text: paragraphs separated by blank lines (single newlines inside a paragraph are joined).
pub fn text_to_blocks(text: &str) -> Vec<Block> {
    let mut b = Builder::new();
    let norm = text.replace("\r\n", "\n");
    for para in norm.split("\n\n") {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        b.start(BlockKind::Para);
        b.text(p);
        b.flush();
    }
    b.blocks
}

// --- Books ------------------------------------------------------------------------------------
pub struct Chapter {
    pub title: String,
    pub spine: usize,
}

pub struct Book {
    pub title: String,
    pub author: String,
    pub spine: Vec<String>, // zip paths of chapter documents, in reading order
    pub toc: Vec<Chapter>,
    zip: Option<Zip>,
    text: Option<String>,
    mem: Option<Vec<Vec<Block>>>,
    /// link targets of an in-memory book (web article): `Run::link` indexes into this
    links: Vec<String>,
    /// zip path of the cover image, if the EPUB declares one
    cover: Option<String>,
}

fn dir_of(p: &str) -> String {
    match p.rfind('/') {
        Some(i) => p[..=i].to_string(),
        None => String::new(),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolve `href` (relative to `base` dir) into a normalized zip path, dropping any #fragment.
fn resolve(base: &str, href: &str) -> String {
    let href = percent_decode(href.split('#').next().unwrap_or(""));
    let mut parts: Vec<&str> = if href.starts_with('/') { vec![] } else { base.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn text_of(toks: &[Tok], open: &str) -> Option<String> {
    let mut inside = false;
    let mut s = String::new();
    for t in toks {
        match t {
            Tok::Open(n, _, false) if n == open => inside = true,
            Tok::Close(n) if n == open && inside => return Some(decode_entities(s.trim())),
            Tok::Text(x) if inside => s.push_str(x),
            _ => {}
        }
    }
    None
}

impl Book {
    /// A book held in memory (articles, Wikipedia pages): one block list per chapter.
    pub fn from_blocks(title: &str, author: &str, chapters: Vec<(String, Vec<Block>)>) -> Book {
        let toc = chapters.iter().enumerate().map(|(i, (t, _))| Chapter { title: t.clone(), spine: i }).collect();
        let spine = (0..chapters.len()).map(|i| format!("mem{i}")).collect();
        let mem = Some(chapters.into_iter().map(|(_, b)| b).collect());
        Book { title: title.into(), author: author.into(), spine, toc, zip: None, text: None, mem, cover: None, links: vec![] }
    }

    /// Attach the link targets the blocks' runs point into (from `html_to_doc`).
    pub fn with_links(mut self, links: Vec<String>) -> Book {
        self.links = links;
        self
    }

    pub fn open(path: &str) -> Result<Book, String> {
        let lower = path.to_lowercase();
        if [".mobi", ".azw3", ".azw", ".prc"].iter().any(|e| lower.ends_with(e)) {
            let m = crate::mobi::load(path)?;
            return Ok(Book::from_blocks(&m.title, &m.author, m.chapters));
        }
        let other = if lower.ends_with(".fb2") || lower.ends_with(".fb2.zip") {
            Some(crate::formats::fb2(path))
        } else if lower.ends_with(".rtf") {
            Some(crate::formats::rtf(path))
        } else if lower.ends_with(".docx") {
            Some(crate::formats::docx(path))
        } else {
            None
        };
        if let Some(r) = other {
            let l = r?;
            return Ok(Book::from_blocks(&l.title, &l.author, l.chapters));
        }
        if lower.ends_with(".html") || lower.ends_with(".htm") || lower.ends_with(".xhtml") {
            let html = String::from_utf8_lossy(&std::fs::read(path).map_err(|e| e.to_string())?).into_owned();
            let toks = tokenize(&html);
            let title = text_of(&toks, "title").filter(|t| !t.is_empty()).unwrap_or_else(||
                std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
            // saved web pages: reading view (drops site navigation, keeps the article)
            let blocks = html_to_doc(&html, true).0;
            return Ok(Book::from_blocks(&title, "", vec![(title.clone(), blocks)]));
        }
        if lower.ends_with(".txt") {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let text = String::from_utf8(bytes.clone())
                .unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect()); // latin-1 fallback
            let title = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            return Ok(Book { title, author: String::new(), spine: vec!["text".into()],
                             toc: vec![Chapter { title: "Text".into(), spine: 0 }], zip: None, text: Some(text), mem: None, cover: None, links: vec![] });
        }
        let mut zip = Zip::open(path)?;
        let container = zip.read_string("META-INF/container.xml").ok_or("missing META-INF/container.xml")?;
        let opf_path = tokenize(&container).into_iter().find_map(|t| match t {
            Tok::Open(n, a, _) if n == "rootfile" => a.get("full-path").cloned(),
            _ => None,
        }).ok_or("no rootfile in container.xml")?;
        let opf = zip.read_string(&opf_path).ok_or("missing OPF package file")?;
        let base = dir_of(&opf_path);
        let toks = tokenize(&opf);
        let title = text_of(&toks, "title").unwrap_or_default();
        let author = text_of(&toks, "creator").unwrap_or_default();
        let mut manifest: HashMap<String, (String, String, String)> = HashMap::new(); // id -> (path, media, props)
        let mut spine_ids = vec![];
        let mut ncx_id = None;
        let mut cover_id = None;
        for t in &toks {
            if let Tok::Open(n, a, _) = t {
                match n.as_str() {
                    "item" => {
                        if let (Some(id), Some(href)) = (a.get("id"), a.get("href")) {
                            manifest.insert(id.clone(), (resolve(&base, href), a.get("media-type").cloned().unwrap_or_default(),
                                                         a.get("properties").cloned().unwrap_or_default()));
                        }
                    }
                    "itemref" => {
                        if a.get("linear").map(String::as_str) != Some("no") {
                            if let Some(id) = a.get("idref") {
                                spine_ids.push(id.clone());
                            }
                        }
                    }
                    "spine" => ncx_id = a.get("toc").cloned(),
                    "meta" if a.get("name").map(String::as_str) == Some("cover") => cover_id = a.get("content").cloned(),
                    _ => {}
                }
            }
        }
        let spine: Vec<String> = spine_ids.iter().filter_map(|id| manifest.get(id).map(|m| m.0.clone())).collect();
        // cover: EPUB3 properties="cover-image", else EPUB2 <meta name="cover" content="id">
        let cover = manifest.values().find(|m| m.2.split_whitespace().any(|p| p == "cover-image")).map(|m| m.0.clone())
            .or_else(|| cover_id.and_then(|id| manifest.get(&id).map(|m| m.0.clone())))
            .filter(|p| { let l = p.to_lowercase(); l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".png") });
        if spine.is_empty() {
            return Err("EPUB has no readable chapters".into());
        }
        // TOC: EPUB3 nav document, else EPUB2 NCX
        let mut entries: Vec<(String, String)> = vec![]; // (title, zip path)
        if let Some((nav, _, _)) = manifest.values().find(|m| m.2.split_whitespace().any(|p| p == "nav")) {
            if let Some(src) = zip.read_string(nav) {
                let nb = dir_of(nav);
                let toks = tokenize(&src);
                let (mut in_toc, mut href, mut text) = (false, None::<String>, String::new());
                for t in &toks {
                    match t {
                        Tok::Open(n, a, _) if n == "nav" => {
                            in_toc = a.get("type").map_or(true, |v| v.contains("toc"));
                        }
                        Tok::Close(n) if n == "nav" => in_toc = false,
                        Tok::Open(n, a, false) if n == "a" && in_toc => {
                            href = a.get("href").map(|h| resolve(&nb, h));
                            text.clear();
                        }
                        Tok::Text(x) if href.is_some() => text.push_str(x),
                        Tok::Close(n) if n == "a" => {
                            if let Some(h) = href.take() {
                                entries.push((decode_entities(text.split_whitespace().collect::<Vec<_>>().join(" ").as_str()), h));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        if entries.is_empty() {
            let ncx = ncx_id.and_then(|id| manifest.get(&id).map(|m| m.0.clone()))
                .or_else(|| manifest.values().find(|m| m.1 == "application/x-dtbncx+xml").map(|m| m.0.clone()));
            if let Some(ncx) = ncx {
                if let Some(src) = zip.read_string(&ncx) {
                    let nb = dir_of(&ncx);
                    let toks = tokenize(&src);
                    let (mut in_text, mut label) = (false, String::new());
                    for t in &toks {
                        match t {
                            Tok::Open(n, _, false) if n == "text" => {
                                in_text = true;
                                label.clear();
                            }
                            Tok::Text(x) if in_text => label.push_str(x),
                            Tok::Close(n) if n == "text" => in_text = false,
                            Tok::Open(n, a, _) if n == "content" => {
                                if let Some(src) = a.get("src") {
                                    entries.push((decode_entities(label.trim()), resolve(&nb, src)));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        let mut toc = vec![];
        for (t, p) in entries {
            if let Some(i) = spine.iter().position(|s| *s == p) {
                if !t.is_empty() && !toc.iter().any(|c: &Chapter| c.spine == i) {
                    toc.push(Chapter { title: t, spine: i });
                }
            }
        }
        if toc.is_empty() {
            toc = (0..spine.len()).map(|i| Chapter { title: format!("Section {}", i + 1), spine: i }).collect();
        }
        Ok(Book { title, author, spine, toc, zip: Some(zip), text: None, mem: None, cover, links: vec![] })
    }

    pub fn chapter_blocks(&mut self, i: usize) -> Vec<Block> {
        if let Some(m) = &self.mem {
            return m.get(i).cloned().unwrap_or_default();
        }
        if let Some(t) = &self.text {
            return text_to_blocks(t);
        }
        let Some(path) = self.spine.get(i).cloned() else { return vec![] };
        match self.zip.as_mut().and_then(|z| z.read_string(&path)) {
            Some(html) => {
                // resolve image sources to zip paths; drop remote ones
                let base = dir_of(&path);
                html_to_blocks(&html).into_iter().filter_map(|mut b| {
                    if b.kind == BlockKind::Image {
                        let src = b.runs[0].text.clone();
                        if src.contains("://") || src.starts_with("data:") {
                            return None;
                        }
                        b.runs[0].text = resolve(&base, &src);
                    }
                    Some(b)
                }).collect()
            }
            None => vec![Block { kind: BlockKind::Para, runs: vec![Run { text: format!("(Could not read {path})"), bold: false, italic: false, link: None }], start: 0 }],
        }
    }

    /// A chapter with links and anchors (EPUB only; other books have none).
    pub fn chapter_doc(&mut self, i: usize) -> Doc {
        let path = self.spine.get(i).cloned().unwrap_or_default();
        let html = if self.mem.is_none() && self.text.is_none() { self.zip.as_mut().and_then(|z| z.read_string(&path)) } else { None };
        match html {
            Some(h) => {
                let base = dir_of(&path);
                let mut d = html_to_full(&h);
                d.blocks.retain(|b| b.kind != BlockKind::Image || {
                    let s = &b.runs[0].text;
                    !(s.contains("://") || s.starts_with("data:"))
                });
                for b in d.blocks.iter_mut().filter(|b| b.kind == BlockKind::Image) {
                    b.runs[0].text = resolve(&base, &b.runs[0].text);
                }
                d
            }
            None => Doc { blocks: self.chapter_blocks(i), links: self.links.clone(), anchors: HashMap::new() },
        }
    }

    /// Resolve a link found in chapter `from` to (chapter index, fragment id).
    pub fn resolve_link(&self, from: usize, href: &str) -> Option<(usize, Option<String>)> {
        if href.contains("://") || href.starts_with("mailto:") {
            return None;
        }
        let (file, frag) = match href.split_once('#') {
            Some((f, g)) => (f, Some(g.to_string())),
            None => (href, None),
        };
        let chapter = if file.is_empty() {
            from
        } else {
            let base = dir_of(self.spine.get(from)?);
            let target = resolve(&base, file);
            self.spine.iter().position(|s| *s == target)?
        };
        Some((chapter, frag))
    }

    /// Raw bytes of a file inside the EPUB (images).
    pub fn file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.zip.as_mut()?.read(path)
    }

    /// The cover image bytes (JPEG/PNG), if any.
    pub fn cover_image(&mut self) -> Option<Vec<u8>> {
        let path = self.cover.clone()?;
        self.zip.as_mut()?.read(&path)
    }

    /// The relative length of each chapter, for progress through the whole book. EPUBs use the
    /// chapter files' sizes (cheap: no parsing); other books count characters.
    pub fn chapter_weights(&mut self) -> Vec<f32> {
        if let Some(z) = &self.zip {
            return self.spine.iter().map(|p| z.size(p).unwrap_or(0) as f32).collect();
        }
        (0..self.spine.len()).map(|i| self.chapter_blocks(i).iter().map(|b| b.chars()).sum::<usize>() as f32).collect()
    }

    /// Title of the TOC entry covering spine item `i`.
    pub fn chapter_title(&self, i: usize) -> String {
        self.toc.iter().rev().find(|c| c.spine <= i).map(|c| c.title.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_view_drops_share_buttons() {
        let html = "<html><body><article><p>Power supply will remain suspended in these places on Friday for \
            maintenance work, the board said in a statement on Wednesday evening.</p>\
            <ul><li>Copy link</li><li>Email</li><li>Facebook</li><li>Twitter</li><li>WhatsApp</li></ul>\
            <ul><li>First real point</li><li>Second real point</li><li>Email</li></ul></article></body></html>";
        let (blocks, _) = html_to_doc(html, true);
        let items: Vec<String> = blocks.iter().filter(|b| b.kind == BlockKind::Item)
            .map(|b| b.runs.iter().map(|r| r.text.as_str()).collect()).collect();
        assert_eq!(items, vec!["First real point", "Second real point", "Email"], "only the share run goes");
    }

    #[test]
    fn entities_near_multibyte_text() {
        // '&' followed by multi-byte characters within the 12-byte lookahead used to panic
        assert_eq!(decode_entities("a & éééééé; b"), "a & éééééé; b");
        assert_eq!(decode_entities("Tōkyō &amp; Ōsaka"), "Tōkyō & Ōsaka");
        assert_eq!(decode_entities("&#x6771;&#20140; &mdash; 東京"), "東京 — 東京");
        assert_eq!(decode_entities("&"), "&");
    }

    #[test]
    fn attributes_with_non_ascii() {
        let a = parse_attrs(r#"title="déjà vu" alt=café href='/wiki/Caf%C3%A9'"#);
        assert_eq!(a.get("title").map(String::as_str), Some("déjà vu"));
        assert_eq!(a.get("alt").map(String::as_str), Some("café"));
        assert_eq!(a.get("href").map(String::as_str), Some("/wiki/Caf%C3%A9"));
    }

    #[test]
    fn quoted_gt_in_attributes() {
        let html = r#"<body><p>A <span data-mw='{"x":"</ref> > {{tpl}}"}'>B</span> C</p></body>"#;
        let blocks = html_to_blocks(html);
        let text: String = blocks[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "A B C");
    }

    #[test]
    fn soft_hyphens_are_removed() {
        let blocks = html_to_blocks("<body><p>ex\u{ad}pect ar&shy;gu&shy;ment</p></body>");
        let text: String = blocks[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "expect argument");
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("Caf%C3%A9"), "Café");
        assert_eq!(percent_decode("100%é"), "100%é"); // '%' followed by a multi-byte char
        assert_eq!(percent_decode("%"), "%");
    }

    #[test]
    fn html_blocks_unicode() {
        let (blocks, links) = html_to_doc("<body><p>東京 &amp; <a href=\"/wiki/Ōsaka\">大阪</a> — à</p></body>", false);
        assert_eq!(links, vec!["/wiki/Ōsaka".to_string()]);
        let text: String = blocks[0].runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "東京 & 大阪 — à");
    }

    #[test]
    fn script_contents_are_not_markup() {
        let html = r#"<body><script>document.write('<script src=x></script'+'>'); var s = "</div><p>";</script><p>Real text</p></body>"#;
        let (blocks, _) = html_to_doc(html, false);
        let text: String = blocks.iter().flat_map(|b| b.runs.iter().map(|r| r.text.clone())).collect();
        assert_eq!(text.trim(), "Real text");
    }

    #[test]
    fn reading_view_picks_the_article() {
        let para = "<p>This is a long sentence of the actual story, with commas, and enough words to count as a paragraph.</p>";
        let html = format!(r#"<body><article class="account-widget"><p>You are logged in</p><p>Subscribe now to read premium stories, editorials and more.</p></article>
            <div class="articlebodycontent"><div>{0}{0}{0}{0}</div></div><div class="related"><p>Other story headline here, read it.</p></div></body>"#, para);
        let (blocks, _) = html_to_doc(&html, true);
        assert_eq!(blocks.len(), 4);
        assert!(blocks[0].runs[0].text.starts_with("This is a long sentence"));
    }

    #[test]
    fn reading_view_falls_back_to_json_ld() {
        let html = r#"<html><head><script type="application/ld+json">{"@type":"NewsArticle","articleBody":"The court said the owner may live alone, peacefully and with dignity, in the house he bought.\u201cAt 85, he is entitled to peace,\u201d the judge observed in the order on Friday.The case concerned a long family dispute over a house in the city that had gone on for years and years without end, through several courts, appeals and hearings, with the family unable to agree on anything at all, until the High Court finally settled it in a detailed judgment that ran to many pages and cited earlier cases."}</script></head>
            <body><nav>Home / Courts</nav><p>Please register.</p></body></html>"#;
        let (blocks, _) = html_to_doc(html, true);
        assert_eq!(blocks.len(), 3, "{:?}", blocks.iter().map(|b| b.runs[0].text.clone()).collect::<Vec<_>>());
        assert!(blocks[1].runs[0].text.starts_with('“'));
    }

    #[test]
    fn leaked_css_is_removed() {
        let t = "He sought relief within two weeks. .desktop-ad { display: block; } .mobile-ad { display: none; } \
                 @media (max-width: 767px) { .desktop-ad { display: none; } } The petitioner had approached the Court. \
                 Section 3.2 of the Act, a #hashtag and an @mention { stay } untouched.";
        let s = strip_css(t);
        assert_eq!(s, "He sought relief within two weeks. The petitioner had approached the Court. \
                       Section 3.2 of the Act, a #hashtag and an @mention { stay } untouched."
                       .split_whitespace().collect::<Vec<_>>().join(" "));
    }
}
