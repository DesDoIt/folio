//! Get Books: browse/search free ebook catalogs and download EPUBs into documents/.
//! Sources: Project Gutenberg (search via Gutendex, files from gutenberg.org or its mirror)
//! and Standard Ebooks (public Atom feeds). Network access goes through `curl`.

use crate::apps::open_book;
use crate::canvas::*;
use crate::epub::{decode_entities, tokenize, Tok};
use crate::input::Gesture;
use crate::json;
use crate::sys::Sys;
use crate::ui::*;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const UA: &str = "KindleLinux/0.2 (e-reader)";

#[derive(Clone)]
pub struct Item {
    title: String,
    author: String,
    info: String,
    urls: Vec<String>, // tried in order
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Gutenberg,
    StandardEbooks,
}

/// GET a URL as text (via curl). Errors are human-readable.
pub fn http_get(url: &str) -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-sSfL", "--compressed", "-m", "40", "-A", UA, url])
        .stdin(Stdio::null()).output()
        .map_err(|e| format!("curl not available: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(friendly_curl_error(&String::from_utf8_lossy(&out.stderr)))
    }
}

/// GET with optional HTTP basic auth (OPDS servers such as Calibre).
pub fn http_get_auth(url: &str, auth: Option<(&str, &str)>) -> Result<String, String> {
    let mut args = vec!["-sSfL".to_string(), "--compressed".into(), "-m".into(), "40".into(), "-A".into(), UA.into()];
    if let Some((u, p)) = auth.filter(|(u, _)| !u.is_empty()) {
        args.push("--anyauth".into());
        args.push("-u".into());
        args.push(format!("{u}:{p}"));
    }
    args.push(url.into());
    let out = Command::new("curl").args(&args).stdin(Stdio::null()).output()
        .map_err(|e| format!("curl not available: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let e = String::from_utf8_lossy(&out.stderr);
        Err(if e.contains("401") { "Wrong username or password for this catalog.".into() } else { friendly_curl_error(&e) })
    }
}

/// Download any ebook (EPUB/CBZ zip, MOBI/AZW3, PDF), with optional basic auth. Checks the content.
pub fn download_book(url: &str, dest: &str, auth: Option<(&str, &str)>) -> Result<(), String> {
    let tmp = format!("{dest}.part");
    let mut args = vec!["-sSfL".to_string(), "-m".into(), "600".into(), "--connect-timeout".into(), "20".into(),
                        "-y".into(), "45".into(), "-Y".into(), "500".into(), "-A".into(), UA.into(), "-o".into(), tmp.clone()];
    if let Some((u, p)) = auth.filter(|(u, _)| !u.is_empty()) {
        args.push("--anyauth".into());
        args.push("-u".into());
        args.push(format!("{u}:{p}"));
    }
    args.push(url.into());
    let out = Command::new("curl").args(&args).stdin(Stdio::null()).output()
        .map_err(|e| format!("curl not available: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let e = friendly_curl_error(&String::from_utf8_lossy(&out.stderr));
        crate::sys::log(&format!("download {url} failed: {e}"));
        return Err(e);
    }
    let head = std::fs::read(&tmp).unwrap_or_default();
    let ok = head.starts_with(b"PK") || head.starts_with(b"%PDF") || head.get(60..68) == Some(b"BOOKMOBI");
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return Err("The server didn't return an ebook file.".into());
    }
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

fn friendly_curl_error(e: &str) -> String {
    let e = e.trim();
    if e.contains("Could not resolve host") {
        "No internet connection (couldn't look up the server). Check Wi-Fi.".into()
    } else if e.contains("error: 503") || e.contains("error: 502") {
        "The book site is temporarily down. Try again later.".into()
    } else if e.contains("too slow") {
        "The download stalled (weak Wi-Fi?).".into()
    } else if e.contains("timed out") {
        "The server took too long to answer.".into()
    } else if e.contains("certificate") || e.contains("SSL") {
        format!("Secure connection failed (is the clock right?): {e}")
    } else if e.is_empty() {
        "Download failed.".into()
    } else {
        e.trim_start_matches("curl: ").to_string()
    }
}

/// Download to `dest` (via a temp file), checking that the result looks like an EPUB (zip).
fn download(urls: &[String], dest: &str) -> Result<(), String> {
    let r = download_inner(urls, dest);
    let _ = std::fs::remove_file(format!("{dest}.hdr"));
    r
}

/// Bytes received so far and the expected size (from the last Content-Length of the response headers).
fn progress(dest: &str) -> (u64, Option<u64>) {
    let got = std::fs::metadata(format!("{dest}.part")).map(|m| m.len()).unwrap_or(0);
    let hdr = std::fs::read_to_string(format!("{dest}.hdr")).unwrap_or_default();
    let last = hdr.rsplit("HTTP/").next().unwrap_or("");
    let total = last.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case("content-length").then(|| v.trim().parse::<u64>().ok()).flatten()
    });
    (got, total.filter(|t| *t > 0))
}

fn download_inner(urls: &[String], dest: &str) -> Result<(), String> {
    let tmp = format!("{dest}.part");
    let mut last = String::from("no download link");
    for url in urls {
        // No --retry: a server under maintenance answers 503 with "Retry-After: 21600", and curl would
        // sleep that long. Instead we fall through to the next mirror; stalled transfers are cut off too.
        let out = Command::new("curl")
            .args(["-sSfL", "-m", "600", "--connect-timeout", "20", "-y", "45", "-Y", "500", "-A", UA,
                   "-D", &format!("{dest}.hdr"), "-o", &tmp, url])
            .stdin(Stdio::null()).output()
            .map_err(|e| format!("curl not available: {e}"))?;
        if out.status.success() {
            let head = std::fs::read(&tmp).map(|b| b.get(..2).map(|h| h.to_vec())).unwrap_or(None);
            if head.as_deref() == Some(b"PK") {
                crate::sys::log(&format!("getbooks: downloaded {url}"));
                return std::fs::rename(&tmp, dest).map_err(|e| e.to_string());
            }
            last = "the server didn't return an EPUB file".into();
        } else {
            last = friendly_curl_error(&String::from_utf8_lossy(&out.stderr));
        }
        crate::sys::log(&format!("getbooks: {url} failed: {last}"));
        let _ = std::fs::remove_file(&tmp);
    }
    Err(last)
}

pub fn urlencode(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        b' ' => "+".into(),
        _ => format!("%{b:02X}"),
    }).collect()
}

/// "Doyle, Arthur Conan" -> "Arthur Conan Doyle"
fn natural_name(n: &str) -> String {
    match n.split_once(", ") {
        Some((last, first)) if !first.contains(',') => format!("{first} {last}"),
        _ => n.to_string(),
    }
}

fn gutenberg(query: Option<&str>) -> Result<Vec<Item>, String> {
    let url = match query {
        Some(q) => format!("https://gutendex.com/books/?search={}", urlencode(q)),
        None => "https://gutendex.com/books/?sort=popular".into(),
    };
    let body = http_get(&url)?;
    let j = json::parse(&body).map_err(|e| format!("Unexpected answer from Gutendex ({e})"))?;
    let mut items = vec![];
    for r in j.get("results").arr() {
        let Some(id) = r.get("id").num().map(|n| n as u64) else { continue };
        let title = r.get("title").str().unwrap_or("Untitled").replace('\n', " ");
        let author = r.get("authors").arr().first().and_then(|a| a.get("name").str()).map(natural_name).unwrap_or_default();
        let dl = r.get("download_count").num().unwrap_or(0.0) as u64;
        let mut urls = vec![];
        if let Some(u) = r.get("formats").get("application/epub+zip").str() {
            urls.push(u.to_string());
        }
        urls.push(format!("https://www.gutenberg.org/ebooks/{id}.epub3.images"));
        urls.push(format!("https://gutenberg.pglaf.org/cache/epub/{id}/pg{id}-images-3.epub"));
        urls.dedup();
        items.push(Item { title, author, info: format!("Project Gutenberg #{id} · {dl} downloads"), urls });
    }
    Ok(items)
}

fn standard_ebooks(query: Option<&str>) -> Result<Vec<Item>, String> {
    let url = match query {
        Some(q) => format!("https://standardebooks.org/feeds/atom/all?query={}&per-page=24", urlencode(q)),
        None => "https://standardebooks.org/feeds/atom/new-releases".into(),
    };
    let body = http_get(&url)?;
    if !body.contains("<feed") {
        return Err("Standard Ebooks didn't return a feed".into());
    }
    let mut items = vec![];
    let mut cur: Option<Item> = None;
    let mut field: Option<&str> = None;
    let mut text = String::new();
    let mut in_author = false;
    for t in tokenize(&body) {
        match t {
            Tok::Open(n, a, sc) => match n.as_str() {
                "entry" => cur = Some(Item { title: String::new(), author: String::new(), info: String::new(), urls: vec![] }),
                "author" => in_author = true,
                "title" if cur.is_some() && !in_author => {
                    field = Some("title");
                    text.clear();
                }
                "name" if in_author && cur.is_some() => {
                    field = Some("name");
                    text.clear();
                }
                "link" => {
                    if let Some(it) = cur.as_mut() {
                        if a.get("rel").map(String::as_str) == Some("enclosure")
                            && a.get("type").map(String::as_str) == Some("application/epub+zip")
                            && it.urls.is_empty() {
                            if let Some(h) = a.get("href") {
                                it.urls.push(h.clone());
                                if let Some(len) = a.get("length").and_then(|l| l.parse::<u64>().ok()) {
                                    it.info = format!("Standard Ebooks · {}", crate::apps::human(len));
                                }
                            }
                        }
                    }
                    let _ = sc;
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

pub fn safe_filename_ext(title: &str, author: &str, ext: &str) -> String {
    let f = safe_filename(title, author);
    format!("{}.{ext}", f.trim_end_matches(".epub"))
}

fn safe_filename(title: &str, author: &str) -> String {
    let base = if author.is_empty() { title.to_string() } else { format!("{title} - {author}") };
    let clean: String = base.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '-' } else { c }).collect();
    let clean: String = clean.chars().take(120).collect();
    format!("{}.epub", clean.trim().trim_end_matches('.'))
}

enum Job {
    List(Result<Vec<Item>, String>),
}

/// One download in progress; shown as a bar in its book's row.
struct Dl {
    dest: String,
    title: String,
    slot: Arc<Mutex<Option<Result<(), String>>>>,
    shown: (u64, Option<u64>),
}

pub struct GetBooks {
    source: Source,
    query: Option<String>,
    items: Vec<Item>,
    pager: Pager,
    job: Option<Arc<Mutex<Option<Job>>>>,
    busy: Option<String>,
    sel: usize,
    last_path: Option<String>,
    downloads: Vec<Dl>,
    last_progress: std::time::Instant,
}

const ID_GUT: u32 = 1_000_001;
const ID_SE: u32 = 1_000_002;
const ID_SEARCH: u32 = 1_000_003;
const ID_LINK: u32 = 1_000_004;
const T_SEARCH: u32 = 1;
const T_DOWNLOAD: u32 = 2;
const T_LINK: u32 = 3;
const T_READ: u32 = 4;
const T_RETRY: u32 = 6;
const T_MORE: u32 = 5;

impl GetBooks {
    pub fn new() -> GetBooks {
        GetBooks { source: Source::Gutenberg, query: None, items: vec![], pager: Pager::new(78), job: None, busy: None,
                   sel: 0, last_path: None, downloads: vec![], last_progress: std::time::Instant::now() }
    }

    fn spawn(&mut self, label: &str, f: impl FnOnce() -> Job + Send + 'static) {
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            let r = f();
            *s2.lock().unwrap() = Some(r);
        });
        self.job = Some(slot);
        self.busy = Some(label.to_string());
    }

    fn start_download(&mut self, title: &str, urls: Vec<String>, dest: String) {
        let slot = Arc::new(Mutex::new(None));
        let (s2, d2) = (slot.clone(), dest.clone());
        std::thread::spawn(move || {
            let r = download(&urls, &d2);
            *s2.lock().unwrap() = Some(r);
        });
        self.downloads.push(Dl { dest, title: title.to_string(), slot, shown: (0, None) });
    }

    fn load(&mut self) {
        let (src, q) = (self.source, self.query.clone());
        self.items.clear();
        self.pager.page = 0;
        self.spawn("Loading…", move || Job::List(match src {
            Source::Gutenberg => gutenberg(q.as_deref()),
            Source::StandardEbooks => standard_ebooks(q.as_deref()),
        }));
    }

    fn dest(&self, sys: &Sys, it: &Item) -> String {
        format!("{}/documents/{}", sys.us, safe_filename(&it.title, &it.author))
    }
}

impl App for GetBooks {
    fn title(&self) -> String {
        "Get Books".into()
    }

    fn on_show(&mut self, _sys: &mut Sys) {
        if self.items.is_empty() && self.job.is_none() {
            self.load();
        }
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        let b = Btn::size(19);
        let gut = self.source == Source::Gutenberg;
        cx.button(Rect::new(a.x0 + 10, a.y0 + 10, a.x0 + 200, a.y0 + 58), "Gutenberg", Some(Hit::App(ID_GUT)), Btn { inverted: gut, ..b });
        cx.button(Rect::new(a.x0 + 206, a.y0 + 10, a.x0 + 396, a.y0 + 58), "Standard Ebooks", Some(Hit::App(ID_SE)), Btn { inverted: !gut, ..b });
        cx.button(Rect::new(a.x0 + 402, a.y0 + 10, a.x0 + 496, a.y0 + 58), "Search", Some(Hit::App(ID_SEARCH)), b);
        cx.button(Rect::new(a.x0 + 502, a.y0 + 10, a.x1 - 10, a.y0 + 58), "More…", Some(Hit::App(ID_LINK)), b);
        let heading = match (&self.query, self.source) {
            (Some(q), _) => format!("Results for “{q}”"),
            (None, Source::Gutenberg) => "Most popular on Project Gutenberg".into(),
            (None, Source::StandardEbooks) => "Newest from Standard Ebooks".into(),
        };
        cx.bold(a.x0 + 16, a.y0 + 72, &heading, 18, BLACK, Anchor::LT);
        let docs = format!("{}/documents", sys.us);
        let elsewhere = self.downloads.iter()
            .filter(|d| !self.items.iter().any(|it| format!("{docs}/{}", safe_filename(&it.title, &it.author)) == d.dest))
            .count();
        if let Some(msg) = &self.busy {
            cx.text(a.x1 - 16, a.y0 + 72, msg, 18, DARK, Anchor::RT);
        } else if elsewhere > 0 {
            cx.text(a.x1 - 16, a.y0 + 72, &format!("Downloading {elsewhere}…"), 18, DARK, Anchor::RT);
        }
        let items = &self.items;
        let downloads = &self.downloads;
        let empty = if self.busy.is_some() { "" } else { "Nothing found" };
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 102, a.x1, a.y1), items.len(), empty, |cx, r, i| {
            let it = &items[i];
            let path = format!("{docs}/{}", safe_filename(&it.title, &it.author));
            let have = std::path::Path::new(&path).exists();
            let t = cx.ellipsize(&it.title, Face::Bold, 21, r.w() - 100);
            cx.bold(r.x0 + 18, r.y0 + 10, &t, 21, BLACK, Anchor::LT);
            let sub = cx.ellipsize(&it.author, Face::Sans, 17, r.w() - 100);
            cx.text(r.x0 + 18, r.y0 + 38, &sub, 17, BLACK, Anchor::LT);
            if let Some(d) = downloads.iter().find(|d| d.dest == path) {
                // progress bar in place of the info line, percentage (or size so far) on the right
                let bar = Rect::new(r.x0 + 18, r.y0 + 60, r.x1 - 100, r.y0 + 70);
                cx.c.round_rect(bar, 4, WHITE, 2, BLACK);
                let (got, total) = d.shown;
                let label = match total {
                    Some(t) => {
                        let frac = (got as f64 / t as f64).clamp(0.0, 1.0);
                        let w = ((bar.w() - 4) as f64 * frac) as i32;
                        if w > 0 {
                            cx.c.fill(Rect::new(bar.x0 + 2, bar.y0 + 2, bar.x0 + 2 + w, bar.y1 - 2), BLACK);
                        }
                        format!("{}%", (frac * 100.0) as u32)
                    }
                    None if got > 0 => crate::apps::human(got),
                    None => "…".into(),
                };
                cx.bold(r.x1 - 18, r.cy(), &label, 18, BLACK, Anchor::RM);
            } else {
                cx.text(r.x0 + 18, r.y0 + 58, &it.info, 13, DARK, Anchor::LT);
                cx.text(r.x1 - 22, r.cy(), if have { "✓" } else { "↓" }, 26, BLACK, Anchor::RM);
            }
            cx.hit(r, Hit::App(i as u32), None, &it.title);
        });
    }

    fn busy(&self) -> bool {
        self.job.is_some() || !self.downloads.is_empty()
    }

    fn poll(&mut self, sys: &mut Sys) -> Cmd {
        // finished downloads
        let mut cmds = vec![];
        let mut i = 0;
        while i < self.downloads.len() {
            let r = self.downloads[i].slot.lock().unwrap().take();
            match r {
                Some(r) => {
                    let d = self.downloads.remove(i);
                    cmds.push(Cmd::Redraw);
                    cmds.push(match r {
                        Ok(()) => {
                            self.last_path = Some(d.dest.clone());
                            let name = std::path::Path::new(&d.dest).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                            Cmd::Dialog(Dialog::confirm(T_READ, "Downloaded", &format!("{name} is in your library."), "Read now")
                                .with_cancel("Keep browsing"))
                        }
                        Err(e) => Cmd::Dialog(Dialog::message("Download failed", &format!("{}\n{e}", d.title))),
                    });
                }
                None => i += 1,
            }
        }
        // progress: at most every 1.5 s, and only when something visibly changed (e-ink)
        if self.last_progress.elapsed().as_millis() >= 1500 {
            let mut changed = false;
            for d in self.downloads.iter_mut() {
                let p = progress(&d.dest);
                let pct = |p: (u64, Option<u64>)| p.1.map(|t| p.0 * 100 / t.max(1));
                if pct(p) != pct(d.shown) || (p.1.is_none() && p.0 / 50_000 != d.shown.0 / 50_000) {
                    changed = true;
                }
                d.shown = p;
            }
            self.last_progress = std::time::Instant::now();
            if changed {
                cmds.push(Cmd::Fast);
            }
        }
        let done = self.job.as_ref().and_then(|j| j.lock().unwrap().take());
        let Some(done) = done else {
            return match cmds.len() {
                0 => Cmd::None,
                1 => cmds.pop().unwrap(),
                _ => Cmd::Batch(cmds),
            };
        };
        self.job = None;
        self.busy = None;
        let done = match done {
            Job::List(Ok(items)) => {
                self.items = items;
                Cmd::Redraw
            }
            Job::List(Err(e)) => Cmd::Batch(vec![Cmd::Redraw,
                Cmd::Dialog(Dialog::confirm(T_RETRY, "Couldn't load books", &e, "Try again"))]),
        };
        let _ = sys;
        cmds.insert(0, done);
        Cmd::Batch(cmds)
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        if self.job.is_some() && id >= ID_GUT && id != ID_LINK {
            return Cmd::Toast("Still loading…".into());
        }
        match id {
            ID_GUT | ID_SE => {
                self.source = if id == ID_GUT { Source::Gutenberg } else { Source::StandardEbooks };
                self.query = None;
                self.load();
                Cmd::Redraw
            }
            ID_SEARCH => Cmd::Dialog(Dialog::input(T_SEARCH, "Search books", self.query.as_deref().unwrap_or(""), "Search")
                .with_message("Title or author")),
            ID_LINK => Cmd::Dialog(Dialog::menu(T_MORE, "More sources",
                vec!["My catalogs (OPDS, Calibre…)".into(), "Download from link".into()])),
            i => {
                let Some(it) = self.items.get(i as usize) else { return Cmd::None };
                self.sel = i as usize;
                let dest = self.dest(sys, it);
                if self.downloads.iter().any(|d| d.dest == dest) {
                    return Cmd::Toast("Already downloading".into());
                }
                if std::path::Path::new(&dest).exists() {
                    return open_book(sys, &dest);
                }
                Cmd::Dialog(Dialog::confirm(T_DOWNLOAD, "Download?", &format!("{}\n{}", it.title, it.author), "Download"))
            }
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match tag {
            T_SEARCH => {
                let q = text.trim().to_string();
                self.query = if q.is_empty() { None } else { Some(q) };
                self.load();
                Cmd::Redraw
            }
            T_DOWNLOAD => {
                let Some(it) = self.items.get(self.sel).cloned() else { return Cmd::None };
                let dest = self.dest(sys, &it);
                if !self.downloads.iter().any(|d| d.dest == dest) {
                    self.start_download(&it.title, it.urls.clone(), dest);
                }
                Cmd::Fast
            }
            T_LINK => {
                let url = text.trim().to_string();
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Cmd::Dialog(Dialog::message("Not a link", "Enter a full http:// or https:// address."));
                }
                let name = url.rsplit('/').next().unwrap_or("download").split('?').next().unwrap_or("download").to_string();
                let name = if name.to_lowercase().ends_with(".epub") { name } else { format!("{name}.epub") };
                let dest = format!("{}/documents/{}", sys.us, name.replace(['\\', ':', '*', '?', '"', '<', '>', '|'], "-"));
                if !self.downloads.iter().any(|d| d.dest == dest) {
                    self.start_download(&name, vec![url], dest);
                }
                Cmd::Fast
            }
            T_MORE => match choice {
                0 => Cmd::Push(Box::new(crate::opds::Catalogs::new())),
                _ => Cmd::Dialog(Dialog::input(T_LINK, "Download from link", "https://", "Download")
                    .with_message("Direct link to an .epub file")),
            },
            T_RETRY => {
                self.load();
                Cmd::Redraw
            }
            T_READ => match self.last_path.clone() {
                Some(p) => open_book(sys, &p),
                None => Cmd::None,
            },
            _ => Cmd::None,
        }
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}
