//! More reflowable formats for the built-in reader: FictionBook (FB2, FB2.ZIP), RTF and Word DOCX.
//! Each is turned into simple HTML and goes through the same converter as EPUB chapters.

use crate::epub::{decode_entities, html_to_blocks, tokenize, Block, Tok, Zip};

pub struct Loaded {
    pub title: String,
    pub author: String,
    pub chapters: Vec<(String, Vec<Block>)>,
}

fn stem(path: &str) -> String {
    let name = std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = name.trim_end_matches(".zip").trim_end_matches(".ZIP");
    std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn chapter_name(blocks: &[Block], n: usize) -> String {
    blocks.iter().find(|b| matches!(b.kind, crate::epub::BlockKind::Heading(_)))
        .map(|b| b.runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .filter(|t| !t.trim().is_empty() && t.chars().count() < 80)
        .unwrap_or_else(|| format!("Part {n}"))
}

fn finish(title: String, author: String, parts: Vec<String>, path: &str) -> Result<Loaded, String> {
    let mut chapters = vec![];
    for p in parts {
        let mut blocks = html_to_blocks(&p);
        blocks.retain(|b| b.kind != crate::epub::BlockKind::Image);
        if blocks.iter().all(|b| b.runs.iter().all(|r| r.text.trim().is_empty())) {
            continue;
        }
        let mut off = 0;
        for b in blocks.iter_mut() {
            b.start = off;
            off += b.chars();
        }
        let name = chapter_name(&blocks, chapters.len() + 1);
        chapters.push((name, blocks));
    }
    if chapters.is_empty() {
        return Err("No readable text found in this file.".into());
    }
    let title = if title.trim().is_empty() { stem(path) } else { title };
    Ok(Loaded { title, author, chapters })
}

// --- FB2 ------------------------------------------------------------------------------------------
pub fn fb2(path: &str) -> Result<Loaded, String> {
    let raw = if path.to_lowercase().ends_with(".zip") {
        let mut z = Zip::open(path)?;
        let name = z.names().into_iter().find(|n| n.to_lowercase().ends_with(".fb2")).ok_or("No .fb2 file inside the zip")?;
        z.read(&name).ok_or("Couldn't unpack the .fb2 file")?
    } else {
        std::fs::read(path).map_err(|e| e.to_string())?
    };
    let head = String::from_utf8_lossy(&raw[..raw.len().min(200)]).to_lowercase();
    let xml = if head.contains("windows-1251") {
        raw.iter().map(|&b| cp1251(b)).collect::<String>()
    } else {
        String::from_utf8_lossy(&raw).into_owned()
    };
    // metadata
    let (mut title, mut first, mut last) = (String::new(), String::new(), String::new());
    let mut field = "";
    let mut in_title_info = false;
    let mut in_author = false;
    for t in tokenize(&xml) {
        match t {
            Tok::Open(n, _, _) => match n.as_str() {
                "title-info" => in_title_info = true,
                "author" if in_title_info && first.is_empty() && last.is_empty() => in_author = true,
                "book-title" if in_title_info => field = "title",
                "first-name" if in_author => field = "first",
                "last-name" if in_author => field = "last",
                "body" => break,
                _ => {}
            },
            Tok::Text(x) => match field {
                "title" => title.push_str(x),
                "first" => first.push_str(x),
                "last" => last.push_str(x),
                _ => {}
            },
            Tok::Close(n) => {
                field = "";
                match n.as_str() {
                    "title-info" => in_title_info = false,
                    "author" => in_author = false,
                    _ => {}
                }
            }
        }
    }
    let author = decode_entities(format!("{} {}", first.trim(), last.trim()).trim());
    // the main body (the first <body>; later ones hold notes), split into top-level sections
    let start = xml.find("<body").ok_or("No <body> in this FB2 file")?;
    let end = xml[start..].find("</body>").map_or(xml.len(), |e| start + e);
    let body = &xml[start..end];
    let mut parts = vec![];
    let mut depth = 0;
    let mut cur = String::new();
    let mut i = 0;
    while i < body.len() {
        let rest = &body[i..];
        if rest.starts_with("<section") {
            if depth == 0 && !cur.trim().is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            depth += 1;
        } else if rest.starts_with("</section>") {
            depth -= 1;
            if depth == 0 {
                cur.push_str("</section>");
                parts.push(std::mem::take(&mut cur));
                i += "</section>".len();
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        cur.push(ch);
        i += ch.len_utf8();
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    let parts = parts.into_iter().map(|p| fb2_to_html(&p)).collect();
    finish(decode_entities(title.trim()), author, parts, path)
}

/// FB2 tags -> HTML the converter understands.
fn fb2_to_html(s: &str) -> String {
    // titles hold <p> lines; flatten them so the heading stays one block
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(a) = rest.find("<title>") {
        out.push_str(&rest[..a]);
        let after = &rest[a..];
        let end = after.find("</title>").map_or(after.len(), |e| e + "</title>".len());
        let t = after[..end].replace("<p>", "").replace("</p>", " ");
        out.push_str(&t);
        rest = &after[end..];
    }
    out.push_str(rest);
    for (a, b) in [("<section", "<div"), ("</section>", "</div>"), ("<title>", "<h2>"), ("</title>", "</h2>"),
                   ("<subtitle>", "<h3>"), ("</subtitle>", "</h3>"), ("<emphasis>", "<em>"), ("</emphasis>", "</em>"),
                   ("<strong>", "<b>"), ("</strong>", "</b>"), ("<epigraph", "<blockquote"), ("</epigraph>", "</blockquote>"),
                   ("<cite", "<blockquote"), ("</cite>", "</blockquote>"), ("<poem", "<div"), ("</poem>", "</div>"),
                   ("<stanza>", "<div>"), ("</stanza>", "</div>"), ("<v>", "<p>"), ("</v>", "</p>"),
                   ("<text-author>", "<p><em>"), ("</text-author>", "</em></p>"), ("<empty-line/>", "<br/>")] {
        out = out.replace(a, b);
    }
    format!("<body>{out}</body>")
}

fn cp1251(b: u8) -> char {
    match b {
        0..=0x7f => b as char,
        0xc0..=0xff => char::from_u32(0x410 + (b as u32 - 0xc0)).unwrap_or('?'),
        0xa8 => 'Ё',
        0xb8 => 'ё',
        0x96 => '–',
        0x97 => '—',
        0x93 => '“',
        0x94 => '”',
        0xab => '«',
        0xbb => '»',
        _ => ' ',
    }
}

// --- RTF ------------------------------------------------------------------------------------------
pub fn rtf(path: &str) -> Result<Loaded, String> {
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    if !raw.starts_with(b"{\\rtf") {
        return Err("Not an RTF file".into());
    }
    let html = rtf_to_html(&raw);
    // split long documents into ~40-paragraph parts so the reader stays quick
    let paras: Vec<&str> = html.split("</p>").filter(|p| !p.trim().is_empty()).collect();
    let parts: Vec<String> = paras.chunks(40).map(|c| format!("<body>{}</p></body>", c.join("</p>"))).collect();
    finish(String::new(), String::new(), parts, path)
}

/// A small RTF reader: text, paragraphs, bold/italic, \'hh and \uN characters. Skips font/colour
/// tables, pictures and other "destinations".
pub fn rtf_to_html(b: &[u8]) -> String {
    let mut out = String::from("<p>");
    // per-group state: (skip, bold, italic, uc)
    let mut stack: Vec<(bool, bool, bool, usize)> = vec![(false, false, false, 1)];
    let mut skip_chars = 0usize;
    let mut i = 0;
    let (mut bold_open, mut italic_open) = (false, false);
    let sync = |out: &mut String, bold: bool, italic: bool, bo: &mut bool, io: &mut bool| {
        if *io && !italic { out.push_str("</i>"); *io = false; }
        if *bo && !bold { out.push_str("</b>"); *bo = false; }
        if bold && !*bo { out.push_str("<b>"); *bo = true; }
        if italic && !*io { out.push_str("<i>"); *io = true; }
    };
    while i < b.len() {
        let st = *stack.last().unwrap();
        match b[i] {
            b'{' => {
                stack.push(st);
                i += 1;
                // {\* ...} and known destinations are skipped
                let rest = &b[i..b.len().min(i + 12)];
                if rest.starts_with(b"\\*") || [&b"\\fonttbl"[..], b"\\colortbl", b"\\stylesheet", b"\\info", b"\\pict",
                    b"\\header", b"\\footer", b"\\listtable", b"\\themedata"].iter().any(|d| rest.starts_with(d)) {
                    stack.last_mut().unwrap().0 = true;
                }
            }
            b'}' => {
                if stack.len() > 1 {
                    stack.pop();
                }
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= b.len() {
                    break;
                }
                let c = b[i];
                if c == b'\'' && i + 2 < b.len() {
                    let v = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("3f"), 16).unwrap_or(b'?');
                    i += 3;
                    if skip_chars > 0 {
                        skip_chars -= 1;
                    } else if !st.0 {
                        sync(&mut out, st.1, st.2, &mut bold_open, &mut italic_open);
                        out.push(cp1252(v));
                    }
                    continue;
                }
                if !c.is_ascii_alphabetic() {
                    // control symbol: \\ \{ \} \~ (nbsp) \- (soft hyphen) \_ (nb hyphen)
                    i += 1;
                    if !st.0 {
                        match c {
                            b'\\' | b'{' | b'}' => out.push(c as char),
                            b'~' => out.push(' '),
                            b'_' => out.push('-'),
                            _ => {}
                        }
                    }
                    continue;
                }
                let s = i;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = std::str::from_utf8(&b[s..i]).unwrap_or("");
                let ns = i;
                if i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let arg: Option<i32> = std::str::from_utf8(&b[ns..i]).ok().and_then(|t| t.parse().ok());
                if i < b.len() && b[i] == b' ' {
                    i += 1;
                }
                let top = stack.last_mut().unwrap();
                match word {
                    "par" | "line" if !top.0 => {
                        if word == "par" {
                            if italic_open { out.push_str("</i>"); italic_open = false; }
                            if bold_open { out.push_str("</b>"); bold_open = false; }
                            out.push_str("</p>\n<p>");
                        } else {
                            out.push_str("<br/>");
                        }
                    }
                    "tab" if !top.0 => out.push(' '),
                    "b" => top.1 = arg != Some(0),
                    "i" => top.2 = arg != Some(0),
                    "plain" => {
                        top.1 = false;
                        top.2 = false;
                    }
                    "uc" => top.3 = arg.unwrap_or(1).max(0) as usize,
                    "u" => {
                        if !top.0 {
                            let v = arg.unwrap_or(63);
                            let v = if v < 0 { v + 65536 } else { v } as u32;
                            let (b1, i1) = (top.1, top.2);
                            sync(&mut out, b1, i1, &mut bold_open, &mut italic_open);
                            out.push(char::from_u32(v).unwrap_or('?'));
                        }
                        skip_chars = stack.last().unwrap().3;
                    }
                    "bin" => i += arg.unwrap_or(0).max(0) as usize,
                    _ => {}
                }
            }
            b'\r' | b'\n' => i += 1,
            c => {
                i += 1;
                if skip_chars > 0 {
                    skip_chars -= 1;
                    continue;
                }
                if !st.0 {
                    sync(&mut out, st.1, st.2, &mut bold_open, &mut italic_open);
                    match c {
                        b'<' => out.push_str("&lt;"),
                        b'&' => out.push_str("&amp;"),
                        _ => out.push(c as char),
                    }
                }
            }
        }
    }
    out.push_str("</p>");
    out
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = ['€', ' ', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', ' ', 'Ž', ' ',
                              ' ', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', ' ', 'ž', 'Ÿ'];
    if (0x80..0xa0).contains(&b) { HIGH[(b - 0x80) as usize] } else { b as char }
}

// --- DOCX -----------------------------------------------------------------------------------------
pub fn docx(path: &str) -> Result<Loaded, String> {
    let mut z = Zip::open(path)?;
    let doc = z.read_string("word/document.xml").ok_or("Not a Word document (no word/document.xml)")?;
    let core = z.read_string("docProps/core.xml").unwrap_or_default();
    let meta = |tag: &str| -> String {
        let toks = tokenize(&core);
        let mut on = false;
        let mut v = String::new();
        for t in toks {
            match t {
                Tok::Open(n, _, _) if n == tag => on = true,
                Tok::Text(x) if on => v.push_str(x),
                Tok::Close(n) if n == tag => break,
                _ => {}
            }
        }
        decode_entities(v.trim())
    };
    let (title, author) = (meta("title"), meta("creator"));
    let mut html = String::new();
    let mut parts = vec![];
    let mut para = String::new();
    let mut heading = 0u8;
    let (mut bold, mut italic, mut in_rpr, mut in_t) = (false, false, false, false);
    let mut list = false;
    let mut paras_in_part = 0;
    for t in tokenize(&doc) {
        match t {
            Tok::Open(n, a, sc) => match n.as_str() {
                "p" if !sc => {
                    para.clear();
                    heading = 0;
                    list = false;
                }
                "pstyle" => {
                    let v = a.get("w:val").or_else(|| a.get("val")).map(|s| s.to_lowercase()).unwrap_or_default();
                    if let Some(d) = v.strip_prefix("heading").and_then(|d| d.trim().parse::<u8>().ok()) {
                        heading = d.clamp(1, 4);
                    } else if v == "title" {
                        heading = 1;
                    }
                }
                "numpr" => list = true,
                "r" if !sc => {
                    bold = false;
                    italic = false;
                }
                "rpr" if !sc => in_rpr = true,
                "b" if in_rpr => bold = a.get("w:val").map_or(true, |v| v != "0" && v != "false"),
                "i" if in_rpr => italic = a.get("w:val").map_or(true, |v| v != "0" && v != "false"),
                "t" if !sc => in_t = true,
                "br" | "cr" => para.push_str("<br/>"),
                "tab" if !in_rpr => para.push(' '),
                _ => {}
            },
            Tok::Text(x) if in_t => {
                let x = x.replace('<', "&lt;");
                match (bold, italic) {
                    (true, true) => para.push_str(&format!("<b><i>{x}</i></b>")),
                    (true, false) => para.push_str(&format!("<b>{x}</b>")),
                    (false, true) => para.push_str(&format!("<i>{x}</i>")),
                    _ => para.push_str(&x),
                }
            }
            Tok::Close(n) => match n.as_str() {
                "rpr" => in_rpr = false,
                "t" => in_t = false,
                "p" => {
                    let tag = if heading > 0 { format!("h{}", heading + 1) } else if list { "li".into() } else { "p".into() };
                    // a new part at each top-level heading (or every 60 paragraphs)
                    if (heading == 1 || paras_in_part >= 60) && !html.trim().is_empty() {
                        parts.push(format!("<body>{}</body>", std::mem::take(&mut html)));
                        paras_in_part = 0;
                    }
                    html.push_str(&format!("<{tag}>{para}</{tag}>\n"));
                    paras_in_part += 1;
                }
                _ => {}
            },
            _ => {}
        }
    }
    if !html.trim().is_empty() {
        parts.push(format!("<body>{html}</body>"));
    }
    finish(title, author, parts, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Loaded) -> String {
        l.chapters.iter().flat_map(|(_, b)| b.iter()).map(|b| b.runs.iter().map(|r| r.text.as_str()).collect::<String>())
            .collect::<Vec<_>>().join("|")
    }

    #[test]
    fn rtf_basics() {
        let h = rtf_to_html(br"{\rtf1\ansi{\fonttbl{\f0 Times;}}{\*\generator x;}\f0 Hello \b bold\b0  caf\'e9 \u8212? end\par Second}");
        assert!(h.contains("Hello <b>bold</b>"), "{h}");
        assert!(h.contains("café — end"), "{h}");
        assert!(h.contains("</p>\n<p>Second"), "{h}");
        assert!(!h.contains("Times") && !h.contains("generator"));
    }

    #[test]
    fn fb2_book() {
        let dir = std::env::temp_dir().join("kos-fb2-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.fb2");
        std::fs::write(&p, r#"<?xml version="1.0" encoding="utf-8"?><FictionBook><description><title-info>
            <author><first-name>Anna</first-name><last-name>Writer</last-name></author><book-title>Night Train</book-title>
            </title-info></description><body><section><title><p>Chapter One</p></title><p>It was <emphasis>late</emphasis>.</p>
            </section><section><title><p>Chapter Two</p></title><p>Morning came.</p></section></body></FictionBook>"#).unwrap();
        let l = fb2(p.to_str().unwrap()).unwrap();
        assert_eq!((l.title.as_str(), l.author.as_str(), l.chapters.len()), ("Night Train", "Anna Writer", 2));
        assert_eq!(l.chapters[1].0, "Chapter Two");
        assert!(text(&l).contains("It was late."));
    }
}
