//! Finding free books: Project Gutenberg (through the Gutendex API), Standard Ebooks, and any OPDS
//! catalog (Calibre's content server, home library servers). Builds the addresses to fetch and
//! parses the answers; the platform does the fetching and downloading with its own network stack.

use crate::epub::{decode_entities, tokenize, Tok};
use crate::feed::resolve_url;

/// A book that can be downloaded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub title: String,
    pub author: String,
    /// a line about it: source, size, popularity
    pub info: String,
    /// download addresses, tried in order (all for the same book)
    pub urls: Vec<String>,
    /// file extension the download will have ("epub", "azw3", ...)
    pub ext: String,
    /// a cover image address, if the source gives one
    pub cover: String,
}

/// Percent-encode a search term for a query string.
pub fn urlencode(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        b' ' => "+".into(),
        _ => format!("%{b:02X}"),
    }).collect()
}

/// "1.4 MB"
pub fn human_size(n: u64) -> String {
    let mut v = n as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if v < 1024.0 || unit == "GB" {
            return if unit == "B" { format!("{v:.0} {unit}") } else { format!("{v:.1} {unit}") };
        }
        v /= 1024.0;
    }
    unreachable!()
}

/// "Doyle, Arthur Conan" -> "Arthur Conan Doyle"
fn natural_name(n: &str) -> String {
    match n.split_once(", ") {
        Some((last, first)) if !first.contains(',') => format!("{first} {last}"),
        _ => n.to_string(),
    }
}

// --- Project Gutenberg -------------------------------------------------------------------------------

/// Gutendex address: popular books, or a search.
pub fn gutenberg_url(query: Option<&str>) -> String {
    match query.map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => format!("https://gutendex.com/books/?search={}", urlencode(q)),
        None => "https://gutendex.com/books/?sort=popular".into(),
    }
}

/// Parse a Gutendex page: the books, and the address of the next page.
pub fn parse_gutendex(body: &str) -> Result<(Vec<Found>, Option<String>), String> {
    let j = crate::json::parse(body).map_err(|e| format!("Unexpected answer from Project Gutenberg ({e})"))?;
    let mut items = vec![];
    for r in j.get("results").arr() {
        let Some(id) = r.get("id").num().map(|n| n as u64) else { continue };
        let title = r.get("title").str().unwrap_or("Untitled").replace('\n', " ").replace("\r", "");
        let author = r.get("authors").arr().first().and_then(|a| a.get("name").str()).map(natural_name).unwrap_or_default();
        let dl = r.get("download_count").num().unwrap_or(0.0) as u64;
        let mut urls = vec![];
        if let Some(u) = r.get("formats").get("application/epub+zip").str() {
            urls.push(u.to_string());
        }
        urls.push(format!("https://www.gutenberg.org/ebooks/{id}.epub3.images"));
        urls.push(format!("https://gutenberg.pglaf.org/cache/epub/{id}/pg{id}-images-3.epub"));
        urls.dedup();
        let cover = r.get("formats").get("image/jpeg").str().unwrap_or_default().to_string();
        let info = format!("Project Gutenberg · {} downloads", group_thousands(dl));
        items.push(Found { title, author, info, urls, ext: "epub".into(), cover });
    }
    let next = j.get("next").str().map(str::to_string);
    Ok((items, next))
}

fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// --- Standard Ebooks ---------------------------------------------------------------------------------

/// Standard Ebooks address: newest releases, or a search.
pub fn standard_ebooks_url(query: Option<&str>) -> String {
    match query.map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => format!("https://standardebooks.org/feeds/atom/all?query={}&per-page=24", urlencode(q)),
        None => "https://standardebooks.org/feeds/atom/new-releases".into(),
    }
}

/// Parse a Standard Ebooks Atom feed.
pub fn parse_standard_ebooks(body: &str) -> Result<Vec<Found>, String> {
    if !body.contains("<feed") {
        return Err("Standard Ebooks didn't return a feed".into());
    }
    let mut items = vec![];
    let mut cur: Option<Found> = None;
    let mut field: Option<&str> = None;
    let mut text = String::new();
    let mut in_author = false;
    for t in tokenize(body) {
        match t {
            Tok::Open(n, a, _) => match n.as_str() {
                "entry" => cur = Some(Found { ext: "epub".into(), ..Default::default() }),
                "author" => in_author = true,
                "title" if cur.is_some() && !in_author => {
                    field = Some("title");
                    text.clear();
                }
                "name" if in_author && cur.is_some() => {
                    field = Some("name");
                    text.clear();
                }
                "thumbnail" => {
                    if let (Some(it), Some(u)) = (cur.as_mut(), a.get("url")) {
                        if it.cover.is_empty() {
                            it.cover = u.clone();
                        }
                    }
                }
                "link" => {
                    if let Some(it) = cur.as_mut() {
                        let rel = a.get("rel").map(String::as_str).unwrap_or("");
                        let typ = a.get("type").map(String::as_str).unwrap_or("");
                        if rel == "enclosure" && typ == "application/epub+zip" && it.urls.is_empty() {
                            if let Some(h) = a.get("href") {
                                it.urls.push(h.clone());
                                it.info = match a.get("length").and_then(|l| l.parse::<u64>().ok()) {
                                    Some(len) => format!("Standard Ebooks · {}", human_size(len)),
                                    None => "Standard Ebooks".into(),
                                };
                            }
                        } else if rel.contains("image") && it.cover.is_empty() {
                            if let Some(h) = a.get("href") {
                                it.cover = h.clone();
                            }
                        }
                    }
                }
                _ => {}
            },
            Tok::Text(x) if field.is_some() => text.push_str(x),
            Tok::Close(n) => match n.as_str() {
                "title" | "name" => {
                    if let (Some(f), Some(it)) = (field.take(), cur.as_mut()) {
                        let v = decode_entities(text.trim());
                        if f == "title" { it.title = v } else if it.author.is_empty() { it.author = v }
                    }
                }
                "author" => in_author = false,
                "entry" => {
                    if let Some(it) = cur.take() {
                        if !it.urls.is_empty() {
                            items.push(it);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    Ok(items)
}

// --- OPDS ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct OpdsEntry {
    pub title: String,
    pub author: String,
    /// a sub-catalog to open (navigation entry)
    pub nav: Option<String>,
    /// (mime type, address), best format first
    pub downloads: Vec<(String, String)>,
    pub cover: String,
}

#[derive(Default, Debug)]
pub struct OpdsFeed {
    pub title: String,
    pub entries: Vec<OpdsEntry>,
    pub next: Option<String>,
    /// a URL template with {searchTerms}, or an OpenSearch description address
    pub search: Option<String>,
}

fn rank(mime: &str) -> u8 {
    match mime {
        m if m.contains("epub") => 0,
        m if m.contains("mobipocket") || m.contains("azw3") || m.contains("x-mobi") => 1,
        m if m.contains("fictionbook") || m.contains("fb2") => 2,
        m if m.contains("pdf") => 3,
        m if m.contains("cbz") || m.contains("comicbook") => 4,
        _ => 9,
    }
}

/// The file extension for a download of this type.
pub fn ext_for(mime: &str, url: &str) -> &'static str {
    match rank(mime) {
        0 => "epub",
        1 => if url.contains("azw3") { "azw3" } else { "mobi" },
        2 => "fb2",
        3 => "pdf",
        4 => "cbz",
        _ => "epub",
    }
}

/// Parse an OPDS (Atom) feed; links are resolved against `base`.
pub fn parse_opds(xml: &str, base: &str) -> OpdsFeed {
    let mut feed = OpdsFeed::default();
    let mut cur: Option<OpdsEntry> = None;
    let (mut field, mut text, mut in_author) = (None::<&str>, String::new(), false);
    let mut thumb_is_small = false;
    for t in tokenize(xml) {
        match t {
            Tok::Open(n, a, sc) => match n.as_str() {
                "entry" => {
                    cur = Some(OpdsEntry::default());
                    thumb_is_small = false;
                }
                "author" => in_author = true,
                "title" if !sc => {
                    field = Some("title");
                    text.clear();
                }
                "name" if in_author && !sc => {
                    field = Some("name");
                    text.clear();
                }
                "link" => {
                    let href = a.get("href").map(|h| resolve_url(base, &decode_entities(h)));
                    let rel = a.get("rel").cloned().unwrap_or_default();
                    let typ = a.get("type").cloned().unwrap_or_default();
                    let Some(href) = href else { continue };
                    match cur.as_mut() {
                        Some(e) => {
                            if rel.contains("image") {
                                // prefer the thumbnail: smaller to fetch for a list
                                let small = rel.contains("thumbnail");
                                if e.cover.is_empty() || (small && !thumb_is_small) {
                                    e.cover = href;
                                    thumb_is_small = small;
                                }
                            } else if rel.contains("opds-spec.org/acquisition") || (rank(&typ) < 9 && rel.is_empty()) {
                                if rank(&typ) < 9 {
                                    e.downloads.push((typ, href));
                                }
                            } else if typ.contains("atom+xml") || typ.contains("opds-catalog") || rel == "subsection" {
                                if e.nav.is_none() {
                                    e.nav = Some(href);
                                }
                            }
                        }
                        None => match rel.as_str() {
                            "next" => feed.next = Some(href),
                            "search" => {
                                if feed.search.is_none() || typ.contains("atom") {
                                    feed.search = Some(href);
                                }
                            }
                            _ => {}
                        },
                    }
                }
                _ => {}
            },
            Tok::Text(x) if field.is_some() => text.push_str(x),
            Tok::Close(n) => match n.as_str() {
                "title" | "name" => {
                    if let Some(f) = field.take() {
                        let v = decode_entities(text.trim());
                        match (cur.as_mut(), f) {
                            (Some(e), "title") => e.title = v,
                            (Some(e), "name") if e.author.is_empty() => e.author = v,
                            (None, "title") if feed.title.is_empty() => feed.title = v,
                            _ => {}
                        }
                    }
                }
                "author" => in_author = false,
                "entry" => {
                    if let Some(mut e) = cur.take() {
                        e.downloads.sort_by_key(|(m, _)| rank(m));
                        if !e.title.is_empty() && (e.nav.is_some() || !e.downloads.is_empty()) {
                            feed.entries.push(e);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    feed
}

impl OpdsEntry {
    /// As a downloadable book (the best format, with the others as fallbacks), if it is one.
    pub fn found(&self, catalog: &str) -> Option<Found> {
        let (mime, url) = self.downloads.first()?;
        let ext = ext_for(mime, url);
        let urls = self.downloads.iter().filter(|(m, u)| ext_for(m, u) == ext).map(|(_, u)| u.clone()).collect();
        Some(Found { title: self.title.clone(), author: self.author.clone(), info: format!("{catalog} · {}", ext.to_uppercase()),
                     urls, ext: ext.into(), cover: self.cover.clone() })
    }
}

/// The Atom search template in an OpenSearch description document, resolved against `base`.
pub fn opensearch_template(desc: &str, base: &str) -> Option<String> {
    let mut found = None;
    for t in tokenize(desc) {
        if let Tok::Open(n, a, _) = t {
            if n == "url" {
                let typ = a.get("type").cloned().unwrap_or_default();
                if let Some(tpl) = a.get("template") {
                    if found.is_none() || typ.contains("atom") {
                        found = Some(resolve_url(base, &decode_entities(tpl)));
                    }
                }
            }
        }
    }
    found
}

/// Fill an OpenSearch template with a query.
pub fn fill_search(template: &str, q: &str) -> String {
    template.replace("{searchTerms}", &urlencode(q)).replace("{startPage?}", "1").replace("{startIndex?}", "0")
        .replace("{count?}", "30").replace("{language?}", "")
}

// --- Wikisource ---------------------------------------------------------------------------------------

/// Wikisource language editions Folio offers: (code, name).
pub const WIKISOURCE_LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"), ("hi", "हिन्दी (Hindi)"), ("bn", "বাংলা (Bengali)"), ("ta", "தமிழ் (Tamil)"),
    ("te", "తెలుగు (Telugu)"), ("kn", "ಕನ್ನಡ (Kannada)"), ("ml", "മലയാളം (Malayalam)"), ("mr", "मराठी (Marathi)"),
    ("gu", "ગુજરાતી (Gujarati)"), ("pa", "ਪੰਜਾਬੀ (Punjabi)"), ("sa", "संस्कृतम् (Sanskrit)"), ("or", "ଓଡ଼ିଆ (Odia)"),
    ("fr", "Français"), ("de", "Deutsch"), ("es", "Español"), ("it", "Italiano"), ("pt", "Português"),
    ("ru", "Русский"), ("pl", "Polski"), ("zh", "中文"), ("ar", "العربية"), ("he", "עברית"),
];

fn wikisource_language(lang: &str) -> &str {
    WIKISOURCE_LANGUAGES.iter().find(|l| l.0 == lang).map(|l| l.1).unwrap_or(lang)
}

/// Search one Wikisource edition: works only (the main namespace), with each page's "this is a
/// list of editions" categories so those can be left out.
pub fn wikisource_search_url(lang: &str, query: &str) -> String {
    // by title: a full-text search finds every anthology that mentions the words
    let q = format!("intitle:{}", query.trim());
    format!("https://{lang}.wikisource.org/w/api.php?action=query&format=json&generator=search&gsrsearch={}\
             &gsrnamespace=0&gsrlimit=50&prop=categories&clcategories=Category:Versions%20pages|Category:Disambiguation%20pages\
             &cllimit=max", urlencode(&q))
}

/// Percent-encode a wiki page title for a URL (spaces become underscores).
fn wiki_title(t: &str) -> String {
    t.replace(' ', "_").bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'(' | b')' | b',' | b'\'' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

/// Wikisource search results as books: chapter pages ("Dracula/Chapter 1") fold into their work,
/// lists of editions are left out, and each work downloads as an EPUB from Wikisource's exporter.
pub fn parse_wikisource(lang: &str, body: &str) -> Result<Vec<Found>, String> {
    let j = crate::json::parse(body).map_err(|e| format!("Unexpected answer from Wikisource ({e})"))?;
    let pages = j.get("query").get("pages");
    let crate::json::Json::Obj(map) = pages else { return Ok(vec![]) }; // no results
    let mut hits: Vec<(f64, String, bool)> = map.iter().map(|(_, p)| {
        let flagged = !p.get("categories").arr().is_empty();
        (p.get("index").num().unwrap_or(999.0), p.get("title").str().unwrap_or("").to_string(), flagged)
    }).collect();
    hits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let flagged: std::collections::HashSet<String> = hits.iter().filter(|h| h.2).map(|h| h.1.clone()).collect();
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for (_, title, _) in hits {
        let work = title.split('/').next().unwrap_or(&title).trim().to_string();
        if work.is_empty() || flagged.contains(&work) || !seen.insert(work.clone()) {
            continue;
        }
        let url = format!("https://ws-export.wmcloud.org/?lang={lang}&format=epub&page={}", wiki_title(&work));
        out.push(Found { title: work, author: String::new(), info: format!("Wikisource · {}", wikisource_language(lang)),
                         urls: vec![url], ext: "epub".into(), cover: String::new() });
    }
    Ok(out)
}

// --- Project Gutenberg Australia ---------------------------------------------------------------------

/// The whole Project Gutenberg Australia catalogue, one text file (newest first).
pub const PGA_INDEX_URL: &str = "https://gutenberg.net.au/gutindex_aus.txt";

/// Parse the Gutenberg Australia catalogue: "Aug 2001 Animal Farm, by George Orwell  [id] 0001A"
/// followed by the book's address. Books download as their HTML edition.
pub fn parse_pga_index(body: &str) -> Vec<Found> {
    let lines: Vec<&str> = body.lines().map(str::trim_end).collect();
    let mut out = vec![];
    for (i, entry) in lines.iter().enumerate() {
        // "Mon YYYY Title, Author   [id] n" (sometimes two spaces after the month)
        let mut words = entry.split_whitespace();
        let (Some(month), Some(year)) = (words.next(), words.next()) else { continue };
        if month.len() != 3 || year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) || !entry.contains('[') {
            continue;
        }
        // optional "[Title: full title]" / "[Author: full name]" lines (brackets not always there), then the address
        let mut next = i + 1;
        let (mut full_title, mut full_author) = (None, None);
        while let Some(l) = lines.get(next).map(|l| l.trim().trim_start_matches('[')) {
            let value = |p: &str| l.strip_prefix(p).map(|v| v.trim().trim_end_matches(']').trim().to_string());
            if let Some(t) = value("Title:") {
                full_title = Some(t);
            } else if let Some(a) = value("Author:") {
                full_author = Some(a);
            } else {
                break;
            }
            next += 1;
        }
        let Some(url) = lines.get(next).map(|l| l.trim()).filter(|l| l.starts_with("http")) else { continue };
        let url = url.split_whitespace().next().unwrap_or(url);
        let rest = entry[entry.find(year).unwrap() + 4..].trim_start();
        let text = rest[..rest.rfind('[').unwrap_or(rest.len())].trim();
        let (title, author) = match text.rsplit_once(", by ").or_else(|| text.rsplit_once(", ")).or_else(|| text.rsplit_once(',')) {
            Some((t, a)) => (t.trim().to_string(), a.trim().to_string()),
            None => (text.to_string(), String::new()),
        };
        let author = full_author.unwrap_or(author);
        // prefer the HTML edition: ".../0100011.txt" -> ".../0100011h.html"; PDFs aren't readable here
        let lower = url.to_lowercase();
        let html = if lower.ends_with(".txt") {
            format!("{}h.html", &url[..url.len() - 4])
        } else if lower.ends_with(".html") || lower.ends_with(".htm") {
            url.to_string()
        } else {
            continue;
        }.replacen("http://", "https://", 1);
        out.push(Found { title: full_title.unwrap_or(title), author, info: "Gutenberg Australia".into(), urls: vec![html],
                         ext: "html".into(), cover: String::new() });
    }
    out
}

// --- Faded Page (Canada) ----------------------------------------------------------------------------

/// One page (250 books) of Faded Page's complete list, starting at `start`.
pub fn fadedpage_list_url(start: usize) -> String {
    format!("https://www.fadedpage.com/allbooks.php?range={start}")
}

/// Parse a Faded Page list page: the books, and how many the whole catalogue has.
pub fn parse_fadedpage_list(html: &str) -> (Vec<Found>, usize) {
    let total = html.find("total books").and_then(|i| html[..i].split_whitespace().last()?.parse().ok()).unwrap_or(0);
    let mut out = vec![];
    for row in html.split("<tr>").skip(1) {
        let author = between(row, "csearch.php?author=", "\"").map(|a| natural_name(&decode_entities(a))).unwrap_or_default();
        let Some(pid) = between(row, "showbook.php?pid=", "\"") else { continue };
        let title = between(row, &format!("pid={pid}\">"), "</a>").map(decode_entities).unwrap_or_default();
        let year = row.rsplit("<td>").next().and_then(|t| t.split("</td>").next()).unwrap_or("").trim().to_string();
        if title.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let info = if year.chars().all(|c| c.is_ascii_digit()) && !year.is_empty() { format!("Faded Page · {year}") } else { "Faded Page".into() };
        out.push(Found { title, author, info, urls: vec![format!("https://www.fadedpage.com/link.php?file={pid}.epub")],
                         ext: "epub".into(), cover: String::new() });
    }
    (out, total)
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)? + i;
    Some(&s[i..j])
}

// --- Internet Archive ---------------------------------------------------------------------------------

/// Internet Archive search, limited to texts with an EPUB that are public domain in the US (published
/// before 1930, or marked public domain), outside the lending library.
pub fn archive_search_url(query: &str) -> String {
    let q = format!("({}) AND mediatype:texts AND format:EPUB AND (date:[0001-01-01 TO 1929-12-31] OR licenseurl:*publicdomain*) \
                     AND -collection:inlibrary AND -collection:printdisabled AND -collection:lendinglibrary", query.trim());
    format!("https://archive.org/advancedsearch.php?q={}&fl[]=identifier&fl[]=title&fl[]=creator&fl[]=date&rows=40&output=json",
            urlencode(&q))
}

pub fn parse_archive(body: &str) -> Result<Vec<Found>, String> {
    let j = crate::json::parse(body).map_err(|e| format!("Unexpected answer from the Internet Archive ({e})"))?;
    let mut out = vec![];
    for d in j.get("response").get("docs").arr() {
        let Some(id) = d.get("identifier").str() else { continue };
        let title = d.get("title").str().unwrap_or("Untitled").to_string();
        let author = d.get("creator").str().map(str::to_string)
            .or_else(|| d.get("creator").arr().first().and_then(|c| c.str()).map(str::to_string))
            .map(|a| natural_name(a.split(',').take(2).collect::<Vec<_>>().join(",").trim_end_matches(|c: char| c == ',' || c.is_ascii_digit() || c == '-').trim()))
            .unwrap_or_default();
        let year = d.get("date").str().map(|s| s.chars().take(4).collect::<String>()).unwrap_or_default();
        out.push(Found {
            title, author,
            info: if year.is_empty() { "Internet Archive".into() } else { format!("Internet Archive · {year}") },
            urls: vec![format!("https://archive.org/download/{id}/{id}.epub")], ext: "epub".into(),
            cover: format!("https://archive.org/services/img/{id}"),
        });
    }
    Ok(out)
}

// --- searching a downloaded catalogue ----------------------------------------------------------------

/// Search a catalogue kept on the device: every word must appear in the title or author; titles that
/// start with the query come first.
pub fn search_catalog(books: &[Found], query: &str, limit: usize) -> Vec<Found> {
    let q = query.trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return books.iter().take(limit).cloned().collect();
    }
    let mut hits: Vec<(u8, &Found)> = books.iter().filter_map(|b| {
        let t = b.title.to_lowercase();
        let hay = format!("{t} {}", b.author.to_lowercase());
        words.iter().all(|w| hay.contains(w)).then(|| (if t.starts_with(&q) { 0 } else if t.contains(&q) { 1 } else { 2 }, b))
    }).collect();
    hits.sort_by_key(|h| h.0);
    hits.into_iter().take(limit).map(|h| h.1.clone()).collect()
}

/// How closely a book matches a search, lower is closer: 0 the title is the query ("Animal Farm",
/// also "Animal Farm: A Fairy Story"), 1 the title starts with it, 2 every word is in the title or
/// every word is in the author, 3 every word is in the title and author together, then fewer words.
pub fn relevance(book: &Found, query: &str) -> u32 {
    fn words(s: &str) -> Vec<String> {
        let w: Vec<String> = s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
        // "The Time Machine" and "Time Machine" are the same search
        match w.first().map(String::as_str) {
            Some("the" | "a" | "an") if w.len() > 1 => w[1..].to_vec(),
            _ => w,
        }
    }
    let q = words(query);
    if q.is_empty() {
        return 0;
    }
    let title = words(&book.title);
    let main = words(book.title.split([':', ';']).next().unwrap_or(""));
    let author = words(&book.author);
    let has = |list: &[String], w: &String| list.iter().any(|x| x == w || (w.len() > 3 && x.starts_with(w.as_str())));
    if main == q || title == q {
        0
    } else if title.starts_with(&q) {
        1
    } else if q.iter().all(|w| has(&title, w)) || q.iter().all(|w| has(&author, w)) {
        2
    } else {
        let found = q.iter().filter(|w| has(&title, w) || has(&author, w)).count();
        3 + (q.len() - found) as u32
    }
}

/// Mix result lists from several sources into one: round-robin in the order given (so every
/// source shows near the top; put the best editions' source first), dropping later copies of a
/// book already listed (same title and author surname), then the closest matches to `query` first
/// (see [`relevance`]; equally close books keep the mixed order). Returns (source index, book).
pub fn merge_results(lists: &[Vec<Found>], query: &str) -> Vec<(usize, Found)> {
    fn norm(s: &str) -> String {
        s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
    }
    let surname = |a: &str| a.split_whitespace().last().unwrap_or("").to_string();
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    let longest = lists.iter().map(|l| l.len()).max().unwrap_or(0);
    for i in 0..longest {
        for (src, list) in lists.iter().enumerate() {
            if let Some(f) = list.get(i) {
                if seen.insert(format!("{}|{}", norm(&f.title), norm(&surname(&f.author)))) {
                    out.push((src, f.clone()));
                }
            }
        }
    }
    out.sort_by_cached_key(|(_, f)| relevance(f, query)); // stable
    out
}

/// A safe file name for a downloaded book.
pub fn file_name(title: &str, author: &str, ext: &str) -> String {
    let base = if author.is_empty() { title.to_string() } else { format!("{title} - {author}") };
    let clean: String = base.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '-' } else { c }).collect();
    let clean: String = clean.chars().take(120).collect();
    format!("{}.{ext}", clean.trim().trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutendex_page() {
        let body = r#"{"count":2,"next":"https://gutendex.com/books/?page=2&sort=popular","results":[
            {"id":1342,"title":"Pride and Prejudice","authors":[{"name":"Austen, Jane"}],"download_count":71234,
             "formats":{"application/epub+zip":"https://www.gutenberg.org/ebooks/1342.epub3.images",
                        "image/jpeg":"https://www.gutenberg.org/cache/epub/1342/pg1342.cover.medium.jpg"}}]}"#;
        let (items, next) = parse_gutendex(body).unwrap();
        assert_eq!(next.as_deref(), Some("https://gutendex.com/books/?page=2&sort=popular"));
        assert_eq!(items[0].author, "Jane Austen");
        assert_eq!(items[0].info, "Project Gutenberg · 71,234 downloads");
        assert_eq!(items[0].urls[0], "https://www.gutenberg.org/ebooks/1342.epub3.images");
        assert_eq!(items[0].urls.len(), 2, "duplicate fallback dropped");
        assert!(items[0].cover.ends_with("cover.medium.jpg"));
        assert_eq!(gutenberg_url(Some("war and peace")), "https://gutendex.com/books/?search=war+and+peace");
    }

    #[test]
    fn standard_ebooks_feed() {
        let body = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:media="http://search.yahoo.com/mrss/"><title>New</title>
            <entry><title>Dracula</title><author><name>Bram Stoker</name></author>
            <media:thumbnail url="https://standardebooks.org/images/covers/dracula-cover.jpg"/>
            <link href="https://standardebooks.org/ebooks/bram-stoker/dracula/downloads/dracula.epub" rel="enclosure" type="application/epub+zip" length="1468006"/>
            </entry></feed>"#;
        let items = parse_standard_ebooks(body).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Dracula");
        assert_eq!(items[0].author, "Bram Stoker");
        assert_eq!(items[0].info, "Standard Ebooks · 1.4 MB");
        assert!(items[0].cover.ends_with("dracula-cover.jpg"));
    }

    #[test]
    fn merging_alternates_sources_and_drops_duplicate_editions() {
        let b = |t: &str, a: &str| Found { title: t.into(), author: a.into(), ..Default::default() };
        let se = vec![b("Dracula", "Bram Stoker"), b("Emma", "Jane Austen")];
        let pg = vec![b("DRACULA!", "B. Stoker"), b("Ulysses", "James Joyce"), b("Persuasion", "Jane Austen")];
        let m = merge_results(&[se, pg], "");
        let got: Vec<(usize, &str)> = m.iter().map(|(s, f)| (*s, f.title.as_str())).collect();
        assert_eq!(got, vec![(0, "Dracula"), (0, "Emma"), (1, "Ulysses"), (1, "Persuasion")]);
    }

    #[test]
    fn closest_matches_come_first() {
        let b = |t: &str, a: &str| Found { title: t.into(), author: a.into(), ..Default::default() };
        let ws = vec![b("St. Nicholas", ""), b("The Golden Bough", "")];
        let pga = vec![b("Animal Farm", "George Orwell")];
        let ia = vec![b("Manual of farm animals", "Merritt Wesley Harper"), b("Animal Farm: A Fairy Story", "Orwell")];
        let m = merge_results(&[ws, pga, ia], "animal farm");
        let got: Vec<&str> = m.iter().map(|(_, f)| f.title.as_str()).collect();
        assert_eq!(got, vec!["Animal Farm", "Animal Farm: A Fairy Story", "Manual of farm animals", "St. Nicholas", "The Golden Bough"]);
        assert_eq!(relevance(&b("The Time Machine", "H. G. Wells"), "time machine"), 0);
        assert_eq!(relevance(&b("A Christmas Carol", "Charles Dickens"), "dickens"), 2);
    }

    #[test]
    fn wikisource_folds_chapters_and_skips_edition_lists() {
        let body = r#"{"query":{"pages":{
            "1":{"index":2,"title":"Dracula/Chapter 1"},
            "2":{"index":1,"title":"Dracula"},
            "3":{"index":3,"title":"Pride and Prejudice","categories":[{"title":"Category:Versions pages"}]},
            "4":{"index":4,"title":"Pride and Prejudice (1813)/Volume 1"}}}}"#;
        let f = parse_wikisource("en", body).unwrap();
        let titles: Vec<&str> = f.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, vec!["Dracula", "Pride and Prejudice (1813)"]);
        assert_eq!(f[0].urls[0], "https://ws-export.wmcloud.org/?lang=en&format=epub&page=Dracula");
        assert_eq!(f[1].urls[0], "https://ws-export.wmcloud.org/?lang=en&format=epub&page=Pride_and_Prejudice_(1813)");
        assert!(parse_wikisource("en", r#"{"batchcomplete":""}"#).unwrap().is_empty(), "no results");
    }

    #[test]
    fn gutenberg_australia_index() {
        let body = "Header text\n\nAug 2001 Animal Farm, by George Orwell                     [010001xx.xxx] 0001A\n\
                    http://gutenberg.net.au/ebooks01/0100011.txt\n\
                    Sep 2024 Green Grey Homestead, Steele Rudd                 [240026xx.xxx] 4553A\n\
                    http://gutenberg.net.au/ebooks24/2400261h.html\n";
        let f = parse_pga_index(body);
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].title.as_str(), f[0].author.as_str()), ("Animal Farm", "George Orwell"));
        assert_eq!(f[0].urls[0], "https://gutenberg.net.au/ebooks01/0100011h.html");
        assert_eq!(f[1].author, "Steele Rudd");
        assert_eq!(search_catalog(&f, "animal orwell", 10).len(), 1);
        assert_eq!(search_catalog(&f, "farm", 10)[0].title, "Animal Farm");
    }

    #[test]
    fn faded_page_list() {
        let html = "<p>All books, rows 1 through 250 of 9225 total books.</p><table>\
            <tr><td><a href=\"/csearch.php?author=Abbott, Edwin Abbott\">Abbott, Edwin Abbott</a></td>\
            <td><a href=\"/showbook.php?pid=20140519\">Flatland--A Romance of Many Dimensions</a></td><td>1884</td></tr></table>";
        let (books, total) = parse_fadedpage_list(html);
        assert_eq!(total, 9225);
        assert_eq!(books[0].author, "Edwin Abbott Abbott");
        assert_eq!(books[0].info, "Faded Page · 1884");
        assert_eq!(books[0].urls[0], "https://www.fadedpage.com/link.php?file=20140519.epub");
    }

    #[test]
    fn internet_archive_results() {
        let body = r#"{"response":{"docs":[{"identifier":"prideprejudice00aust","title":"Pride and prejudice","creator":"Austen, Jane, 1775-1817","date":"1892-01-01T00:00:00Z"}]}}"#;
        let f = parse_archive(body).unwrap();
        assert_eq!(f[0].author, "Jane Austen");
        assert_eq!(f[0].info, "Internet Archive · 1892");
        assert_eq!(f[0].urls[0], "https://archive.org/download/prideprejudice00aust/prideprejudice00aust.epub");
        assert!(archive_search_url("dracula").contains("1929-12-31"));
    }

    #[test]
    fn opds_navigation_and_books() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/sample-opds-books.xml");
        let xml = std::fs::read_to_string(path).unwrap();
        let f = parse_opds(&xml, "https://library.example/opds/books");
        assert!(!f.entries.is_empty());
        let book = f.entries.iter().find(|e| !e.downloads.is_empty()).expect("a downloadable entry");
        let found = book.found("Home").unwrap();
        assert!(found.urls[0].starts_with("http"), "{:?}", found.urls);
        assert_eq!(fill_search("https://x/s?q={searchTerms}&p={startPage?}", "a b"), "https://x/s?q=a+b&p=1");
    }
}

