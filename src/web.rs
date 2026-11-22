//! Touch web browser in "reading mode": pages are fetched with curl, converted to text blocks,
//! laid out like the book reader, with tappable underlined links. No JavaScript.
//! EPUB links download into the library; any page can be handed to Amazon's browser.

use crate::apps::open_book;
use crate::canvas::*;
use crate::epub::{html_to_doc, Block};
use crate::input::{Gesture, Kind};
use crate::reader::{draw_page, paginate, Page};
use crate::sys::Sys;
use crate::ui::*;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const UA: &str = "Mozilla/5.0 (Linux; Android 10; e-reader) AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36 KindleLinux/0.2";
const START: &str = "about:start";

/// Join a link with the page URL it appeared on.
pub fn url_join(base: &str, href: &str) -> String {
    let href = href.trim();
    // DuckDuckGo Lite wraps results in a redirect: //duckduckgo.com/l/?uddg=<encoded target>
    if href.contains("duckduckgo.com/l/?") {
        if let Some(enc) = href.split(['?', '&']).find_map(|kv| kv.strip_prefix("uddg=")) {
            return percent_decode(enc);
        }
    }
    if href.contains("://") || href.starts_with("about:") || href.starts_with("mailto:") {
        return href.to_string();
    }
    let scheme = base.split("://").next().unwrap_or("https");
    if let Some(rest) = href.strip_prefix("//") {
        return format!("{scheme}:{}", "//".to_string() + rest);
    }
    let after = base.split("://").nth(1).unwrap_or("");
    let host = after.split('/').next().unwrap_or("");
    let origin = format!("{scheme}://{host}");
    if href.starts_with('/') {
        return origin + href;
    }
    let path = &after[host.len()..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    if href.starts_with('?') {
        return format!("{origin}{path}{href}");
    }
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in href.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let trailing = if href.ends_with('/') { "/" } else { "" };
    format!("{origin}/{}{trailing}", parts.join("/"))
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3]).map_err(|_| ()).and_then(|h| u8::from_str_radix(h, 16).map_err(|_| ())) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
        b' ' => "+".into(),
        _ => format!("%{b:02X}"),
    }).collect()
}

pub const ENGINES: [(&str, &str); 3] = [
    ("DuckDuckGo", "https://html.duckduckgo.com/html/?q="),
    ("Marginalia", "https://old-search.marginalia.nu/search?query="),
    ("Wikipedia", "https://en.m.wikipedia.org/w/index.php?fulltext=1&search="),
];

fn engine_index(sys: &Sys) -> usize {
    ENGINES.iter().position(|(n, _)| *n == sys.config.get("web_engine")).unwrap_or(0)
}

/// What the user typed in the address bar -> URL (search if it doesn't look like an address).
pub fn address_to_url(input: &str, engine: usize) -> String {
    let t = input.trim();
    if t.contains("://") {
        t.to_string()
    } else if !t.contains(' ') && t.contains('.') {
        format!("https://{t}")
    } else {
        format!("{}{}", ENGINES[engine.min(ENGINES.len() - 1)].1, urlencode(t))
    }
}

/// `<meta http-equiv="refresh" content="2; url=...">` with a short delay -> target URL.
fn meta_refresh(html: &str, base: &str) -> Option<String> {
    let l = html.to_ascii_lowercase();
    let i = l.find("http-equiv=\"refresh\"").or_else(|| l.find("http-equiv='refresh'"))?;
    let tag_start = l[..i].rfind('<')?;
    let tag_end = i + l[i..].find('>')?;
    let tag = &html[tag_start..tag_end];
    let c = tag.to_ascii_lowercase().find("content=")?;
    let rest = &tag[c + 8..];
    let q = rest.chars().next()?;
    let val: String = rest[1..].chars().take_while(|&ch| ch != q).collect();
    let (delay, target) = val.split_once(';')?;
    if delay.trim().parse::<f32>().ok()? > 5.0 {
        return None;
    }
    let t = target.trim();
    let t = t.strip_prefix("url=").or_else(|| t.strip_prefix("URL=")).unwrap_or(t).trim_matches(['\'', '"']);
    if t.is_empty() { None } else { Some(url_join(base, &crate::epub::decode_entities(t))) }
}

/// Tiny pages that bounce with `location.replace('...')` / `location.href = '...'` after a pause.
fn js_redirect(html: &str, base: &str) -> Option<String> {
    if html.len() > 30_000 {
        return None;
    }
    for pat in ["location.replace(", "location.href=", "location.href =", "window.location=", "window.location ="] {
        if let Some(i) = html.find(pat) {
            let rest = html[i + pat.len()..].trim_start();
            let q = rest.chars().next()?;
            if q != '\'' && q != '"' {
                continue;
            }
            let target: String = rest[1..].chars().take_while(|&c| c != q).collect();
            if !target.is_empty() {
                return Some(url_join(base, &crate::epub::decode_entities(&target)));
            }
        }
    }
    None
}

/// Bot-check pages need a real browser with JavaScript/images; we can only explain them.
/// Judged on the *visible* text of short pages: big pages (e.g. Wikipedia) mention "captcha" in scripts.
fn is_challenge(html: &str, blocks: &[Block]) -> bool {
    let visible: String = blocks.iter().flat_map(|b| b.runs.iter().map(|r| r.text.as_str())).collect::<Vec<_>>().join(" ");
    if visible.chars().count() > 2500 {
        return false;
    }
    let v = visible.to_lowercase();
    let title = title_of(html).to_lowercase();
    v.contains("captcha") || v.contains("bots use duckduckgo") || v.contains("confirm this search was made by a human")
        || v.contains("verify you are human") || v.contains("are you a robot") || title.starts_with("just a moment")
        || v.contains("enable javascript and cookies to continue")
}

enum Fetched {
    Page { url: String, html: String },
    File { url: String, path: String },
    Error(String),
}

fn fetch(url: &str, tmp_dir: &str) -> Fetched {
    let tmp = format!("{tmp_dir}/kos-web-{}.tmp", std::process::id());
    let out = Command::new("curl")
        .args(["-sSL", "--compressed", "-m", "45", "--max-filesize", "30000000", "-A", UA, "-o", &tmp,
               "-w", "%{url_effective}\n%{content_type}\n%{http_code}", url])
        .stdin(Stdio::null()).output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return Fetched::Error(format!("curl not available: {e}")),
    };
    let meta = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut m = meta.lines();
    let (eff, ctype, code) = (m.next().unwrap_or(url).to_string(), m.next().unwrap_or("").to_lowercase(), m.next().unwrap_or("0").to_string());
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let e = String::from_utf8_lossy(&out.stderr);
        return Fetched::Error(if e.contains("resolve host") { "No internet connection. Check Wi-Fi.".into() } else { e.trim().trim_start_matches("curl: ").to_string() });
    }
    let is_book = ctype.contains("epub") || eff.to_lowercase().split('?').next().unwrap_or("").ends_with(".epub");
    if is_book {
        return Fetched::File { url: eff, path: tmp };
    }
    let bytes = std::fs::read(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    if !(ctype.is_empty() || ctype.contains("html") || ctype.contains("text") || ctype.contains("xml")) {
        return Fetched::Error(format!("This link is a file ({ctype}) that can't be shown."));
    }
    if code.starts_with('4') || code.starts_with('5') {
        if bytes.is_empty() {
            return Fetched::Error(format!("The server answered with error {code}."));
        }
    }
    Fetched::Page { url: eff, html: String::from_utf8_lossy(&bytes).into_owned() }
}

fn title_of(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(s) = lower.find("<title") else { return String::new() };
    let Some(gt) = lower[s..].find('>') else { return String::new() };
    let start = s + gt + 1;
    let end = lower[start..].find("</title").map_or(start, |e| start + e);
    crate::epub::decode_entities(html[start..end].split_whitespace().collect::<Vec<_>>().join(" ").as_str())
}

pub struct Web {
    url: String,
    title: String,
    html: String,
    blocks: Vec<Block>,
    links: Vec<String>,
    pages: Vec<Page>,
    page: usize,
    laid_out: Option<(Rect, u16)>,
    history: Vec<(String, usize)>,
    reader_view: bool,
    fs: u16,
    job: Option<Arc<Mutex<Option<Fetched>>>>,
    loading: Option<String>,
    link_boxes: Vec<(Rect, u32)>,
    saved_book: Option<String>,
    last_query: Option<String>,
    redirects: u32,
    pending: Option<(std::time::Instant, String)>,
}

const ID_BACK: u32 = 1;
const ID_ADDR: u32 = 2;
const ID_MENU: u32 = 3;
const ID_PREV: u32 = 4;
const ID_NEXT: u32 = 5;
const ID_LINK: u32 = 0x4000_0000;
const T_ADDR: u32 = 1;
const T_MENU: u32 = 2;
const T_READ: u32 = 3;
const T_SIZE: u32 = 4;
const T_ENGINE: u32 = 5;
const T_CHALLENGE: u32 = 6;

fn bookmarks_path(sys: &Sys) -> String {
    format!("{}/kos/bookmarks.txt", sys.us)
}

fn start_page(sys: &Sys) -> String {
    let mut h = String::from("<html><head><title>Start</title></head><body><h1>Kindle Linux Web</h1>\
        <p>Tap the address bar above to search the web or type an address. Pages are shown as readable \
        text with underlined links; tap a link to follow it. Swipe or use the buttons below to turn pages.</p>\
        <h2>Bookmarks</h2>");
    let bm = std::fs::read_to_string(bookmarks_path(sys)).unwrap_or_default();
    if bm.trim().is_empty() {
        h.push_str("<p>No bookmarks yet. Use ☰ → Bookmark this page.</p>");
    }
    for line in bm.lines() {
        if let Some((url, title)) = line.split_once('\t') {
            h.push_str(&format!("<li><a href=\"{url}\">{title}</a></li>"));
        }
    }
    h.push_str("<h2>Good text-friendly sites</h2>\
        <li><a href=\"https://html.duckduckgo.com/html/\">DuckDuckGo (search)</a></li>\
        <li><a href=\"https://en.m.wikipedia.org/wiki/Special:Random\">Wikipedia: random article</a></li>\
        <li><a href=\"https://text.npr.org/\">NPR text-only news</a></li>\
        <li><a href=\"https://lite.cnn.com/\">CNN lite</a></li>\
        <li><a href=\"https://standardebooks.org/ebooks\">Standard Ebooks</a></li>\
        <li><a href=\"https://www.gutenberg.org/ebooks/search/?sort_order=downloads\">Project Gutenberg top books</a></li>\
        <li><a href=\"https://news.ycombinator.com/\">Hacker News</a></li>\
        </body></html>");
    h
}

impl Web {
    pub fn new(sys: &Sys) -> Web {
        let mut w = Web { url: START.into(), title: "Start".into(), html: String::new(), blocks: vec![], links: vec![],
                          pages: vec![], page: 0, laid_out: None, history: vec![], reader_view: false,
                          fs: sys.config.get("web_font").parse().unwrap_or(18), job: None, loading: None,
                          link_boxes: vec![], saved_book: None, last_query: None, redirects: 0, pending: None };
        let html = start_page(sys);
        w.show(START.into(), html);
        w
    }

    /// Open straight at an address (a link tapped in a book or article).
    pub fn open(url: String, sys: &Sys) -> Web {
        let mut w = Web::new(sys);
        w.go(url, sys, false);
        w
    }

    fn show(&mut self, url: String, html: String) {
        self.title = if url == START { "Start".into() } else { title_of(&html) };
        if self.title.is_empty() {
            self.title = url.clone();
        }
        let (blocks, links) = html_to_doc(&html, self.reader_view);
        self.blocks = blocks;
        self.links = links;
        self.html = html;
        self.url = url;
        self.page = 0;
        self.laid_out = None;
    }

    fn go(&mut self, url: String, sys: &Sys, push_history: bool) -> Cmd {
        if push_history {
            self.history.push((self.url.clone(), self.page));
        }
        if url == START {
            let h = start_page(sys);
            self.show(url, h);
            return Cmd::Full;
        }
        let slot = Arc::new(Mutex::new(None));
        let (s2, u2, tmp) = (slot.clone(), url.clone(), if sys.sim { "/tmp".to_string() } else { "/tmp".to_string() });
        std::thread::spawn(move || {
            let r = fetch(&u2, &tmp);
            *s2.lock().unwrap() = Some(r);
        });
        self.job = Some(slot);
        self.loading = Some(url);
        Cmd::Redraw
    }
}

impl App for Web {
    fn back(&mut self, sys: &mut Sys) -> Option<Cmd> {
        if self.history.is_empty() { None } else { Some(self.action(ID_BACK, false, sys)) }
    }

    fn title(&self) -> String {
        "Web".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        // toolbar
        let b = Btn::size(20);
        cx.button(Rect::new(a.x0 + 8, a.y0 + 6, a.x0 + 70, a.y0 + 54), "◀", Some(Hit::App(ID_BACK)),
                  Btn { disabled: self.history.is_empty(), ..b });
        let addr = Rect::new(a.x0 + 78, a.y0 + 6, a.x1 - 78, a.y0 + 54);
        cx.c.round_rect(addr, 8, WHITE, 2, BLACK);
        let shown = match &self.loading {
            Some(u) => format!("Loading {u}"),
            None if self.url == START => "Search or type an address".to_string(),
            None => self.url.trim_start_matches("https://").trim_start_matches("http://").to_string(),
        };
        let shown = cx.ellipsize(&shown, Face::Sans, 17, addr.w() - 20);
        cx.text(addr.x0 + 10, addr.cy(), &shown, 17, if self.loading.is_some() { DARK } else { BLACK }, Anchor::LM);
        cx.hit(addr, Hit::App(ID_ADDR), None, "address");
        cx.button(Rect::new(a.x1 - 70, a.y0 + 6, a.x1 - 8, a.y0 + 54), "☰", Some(Hit::App(ID_MENU)), Btn::size(24));
        // content
        let content = Rect::new(a.x0, a.y0 + 52, a.x1, a.y1 - 58);
        if self.laid_out != Some((content, self.fs)) {
            self.pages = paginate(&self.blocks, cx.f, self.fs, content, crate::reader::Layout::default(),
                                  &crate::reader::Images::new());
            self.page = self.page.min(self.pages.len().saturating_sub(1));
            self.laid_out = Some((content, self.fs));
        }
        // page turning: whole content area swipes; links are registered on top
        // page turns only from the outer edges, so a slightly missed link doesn't flip the page
        let edge = content.w() * 15 / 100;
        cx.hit(Rect::new(content.x1 - edge, content.y0, content.x1, content.y1), Hit::App(ID_NEXT), None, "content");
        cx.hit(Rect::new(content.x0, content.y0, content.x0 + edge, content.y1), Hit::App(ID_PREV), None, "content-left");
        self.link_boxes = match self.pages.get(self.page) {
            Some(p) => draw_page(cx, p).links,
            None => vec![],
        };
        for (r, l) in self.link_boxes.clone() {
            cx.hit(r, Hit::App(ID_LINK | l), None, "link");
        }
        // bottom bar
        let by = a.y1 - 54;
        let n = self.pages.len().max(1);
        cx.button(Rect::new(a.x0 + 12, by, a.x0 + 160, by + 46), "◀ Prev", Some(Hit::App(ID_PREV)),
                  Btn { disabled: self.page == 0, ..Btn::size(19) });
        let t = cx.ellipsize(&self.title, Face::Sans, 15, 240);
        cx.text(a.cx(), by + 14, &t, 15, DARK, Anchor::MM);
        cx.text(a.cx(), by + 34, &format!("{} / {}", self.page + 1, n), 15, DARK, Anchor::MM);
        cx.button(Rect::new(a.x1 - 160, by, a.x1 - 12, by + 46), "Next ▶", Some(Hit::App(ID_NEXT)),
                  Btn { disabled: self.page + 1 >= n, ..Btn::size(19) });
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.pending.is_some()
    }

    fn poll(&mut self, sys: &mut Sys) -> Cmd {
        if let Some((when, _)) = &self.pending {
            if std::time::Instant::now() >= *when {
                let (_, next) = self.pending.take().unwrap();
                return self.go(next, sys, false);
            }
        }
        let done = self.job.as_ref().and_then(|j| j.lock().unwrap().take());
        let Some(done) = done else { return Cmd::None };
        self.job = None;
        self.loading = None;
        match done {
            Fetched::Page { url, html } => {
                if let Some(next) = meta_refresh(&html, &url).or_else(|| js_redirect(&html, &url)).filter(|n| *n != url) {
                    if self.redirects < 3 {
                        self.redirects += 1;
                        // be polite: these pages usually ask for a short wait first
                        self.pending = Some((std::time::Instant::now() + std::time::Duration::from_millis(2200), next));
                        self.show(url, html);
                        return Cmd::Full;
                    }
                }
                self.redirects = 0;
                self.show(url, html);
                let challenge = is_challenge(&self.html, &self.blocks);
                if challenge {
                    let other = if engine_index(sys) == 1 { "DuckDuckGo" } else { "Marginalia" };
                    return Cmd::Batch(vec![Cmd::Full, Cmd::Dialog(Dialog::confirm(T_CHALLENGE, "Blocked by a bot check",
                        &format!("This site wants proof you're human (a puzzle or JavaScript check), which this text \
                                  browser can't do. ☰ → Open in Kindle's browser can. Or search with {other} instead?"),
                        "Use other engine"))]);
                }
                Cmd::Full
            }
            Fetched::File { url, path } => {
                let name = url.split('?').next().unwrap_or("book").rsplit('/').next().unwrap_or("book").to_string();
                let name = if name.to_lowercase().ends_with(".epub") { name } else { format!("{name}.epub") };
                let dest = format!("{}/documents/{}", sys.us, name.replace(['\\', ':', '*', '?', '"', '<', '>', '|'], "-"));
                let moved = std::fs::rename(&path, &dest).or_else(|_| std::fs::copy(&path, &dest).map(|_| ()));
                let _ = std::fs::remove_file(&path);
                if let Some((u, p)) = self.history.pop() {
                    let _ = (u, p); // stay on the page that had the link
                }
                match moved {
                    Ok(()) => {
                        self.saved_book = Some(dest);
                        Cmd::Dialog(Dialog::confirm(T_READ, "Saved to your library", &name, "Read now").with_cancel("Keep browsing"))
                    }
                    Err(e) => Cmd::Dialog(Dialog::message("Couldn't save the book", &e.to_string())),
                }
            }
            Fetched::Error(e) => {
                self.history.pop();
                Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Couldn't open page", &e))])
            }
        }
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        if id & ID_LINK != 0 && id != ID_LINK - 1 {
            let i = (id & !ID_LINK) as usize;
            if let Some(href) = self.links.get(i).cloned() {
                let url = url_join(&self.url, &href);
                if url.starts_with("mailto:") {
                    return Cmd::Toast("Email links aren't supported".into());
                }
                return self.go(url, sys, true);
            }
            return Cmd::None;
        }
        match id {
            ID_BACK => match self.history.pop() {
                Some((u, p)) => {
                    let c = self.go(u, sys, false);
                    self.page = p;
                    c
                }
                None => Cmd::None,
            },
            ID_ADDR => {
                // start empty: clearing a long URL with backspace on e-ink is painful
                let d = Dialog::input(T_ADDR, "Search or address", "", "Go");
                Cmd::Dialog(if self.url == START { d } else { d.with_message(&format!("Now: {}", self.url)) })
            }
            ID_MENU => Cmd::Dialog(Dialog::menu(T_MENU, "Web", vec![
                if self.reader_view { "Reading view: ON".into() } else { "Reading view: OFF".into() },
                "Bookmark this page".into(),
                "Save page for offline reading".into(),
                "Open in Kindle's browser".into(),
                "Text size".into(),
                "Start page".into(),
                format!("Search engine: {}", ENGINES[engine_index(sys)].0),
            ])),
            ID_NEXT => {
                if self.page + 1 < self.pages.len() {
                    self.page += 1;
                    Cmd::Redraw
                } else {
                    Cmd::None
                }
            }
            ID_PREV => {
                self.page = self.page.saturating_sub(1);
                Cmd::Redraw
            }
            _ => Cmd::None,
        }
    }

    fn gesture(&mut self, g: &Gesture, sys: &mut Sys) -> Cmd {
        match g.kind {
            Kind::SwipeLeft | Kind::SwipeUp => self.action(ID_NEXT, false, sys),
            Kind::SwipeRight | Kind::SwipeDown => {
                if self.page == 0 { Cmd::Fast } else { self.action(ID_PREV, false, sys) }
            }
            _ => Cmd::None,
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match (tag, choice) {
            (T_ADDR, _) if !text.trim().is_empty() => {
                self.last_query = Some(text.trim().to_string());
                let u = address_to_url(&text, engine_index(sys));
                self.go(u, sys, true)
            }
            (T_CHALLENGE, _) => {
                let next = if engine_index(sys) == 1 { 0 } else { 1 };
                sys.config.set("web_engine", ENGINES[next].0);
                match self.last_query.clone() {
                    Some(q) => {
                        self.history.pop();
                        let u = address_to_url(&q, next);
                        self.go(u, sys, true)
                    }
                    None => Cmd::Toast(format!("Search engine: {}", ENGINES[next].0)),
                }
            }
            (T_ENGINE, i) => {
                if let Some((n, _)) = ENGINES.get(i) {
                    sys.config.set("web_engine", n);
                }
                Cmd::Toast(format!("Search engine: {}", ENGINES[i.min(ENGINES.len() - 1)].0))
            }
            (T_MENU, 0) => {
                self.reader_view = !self.reader_view;
                let (u, h) = (self.url.clone(), self.html.clone());
                let p = self.page;
                self.show(u, h);
                self.page = p;
                Cmd::Full
            }
            (T_MENU, 1) => {
                if self.url == START {
                    return Cmd::None;
                }
                let line = format!("{}\t{}\n", self.url, self.title.replace('\t', " "));
                let path = bookmarks_path(sys);
                let mut cur = std::fs::read_to_string(&path).unwrap_or_default();
                if !cur.contains(&format!("{}\t", self.url)) {
                    cur.push_str(&line);
                    let _ = std::fs::create_dir_all(format!("{}/kos", sys.us));
                    let _ = std::fs::write(&path, cur);
                }
                Cmd::Toast("Bookmarked".into())
            }
            (T_MENU, 2) => {
                if self.url == START {
                    return Cmd::None;
                }
                let dir = format!("{}/documents/Saved pages", sys.us);
                let _ = std::fs::create_dir_all(&dir);
                let name: String = self.title.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c }).take(80).collect();
                let path = format!("{dir}/{}.html", name.trim());
                match std::fs::write(&path, &self.html) {
                    Ok(()) => Cmd::Toast("Saved to Books → Saved pages".into()),
                    Err(e) => Cmd::Toast(e.to_string()),
                }
            }
            (T_MENU, 3) => {
                sys.request("browser", if self.url == START { "" } else { &self.url });
                Cmd::Quit(11)
            }
            (T_MENU, 4) => Cmd::Dialog(Dialog::menu(T_SIZE, "Text size",
                [14u16, 16, 18, 20, 22, 26].iter().map(|s| format!("{s} px{}", if *s == self.fs { "  ✓" } else { "" })).collect())),
            (T_MENU, 5) => self.go(START.into(), sys, true),
            (T_MENU, 6) => Cmd::Dialog(Dialog::menu(T_ENGINE, "Search engine",
                ENGINES.iter().map(|(n, _)| n.to_string()).collect())),
            (T_SIZE, i) => {
                if let Some(s) = [14u16, 16, 18, 20, 22, 26].get(i) {
                    self.fs = *s;
                    self.laid_out = None;
                    sys.config.set("web_font", &s.to_string());
                }
                Cmd::Full
            }
            (T_READ, _) => match self.saved_book.clone() {
                Some(p) => open_book(sys, &p),
                None => Cmd::None,
            },
            _ => Cmd::None,
        }
    }

    fn persistent_key(&self) -> Option<&'static str> {
        Some("web")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_redirects_with_unicode() {
        // 'İ' lowercases to two chars in Unicode; offsets must still line up
        let html = "<html><head><title>İstanbul — Vikipedi</title><meta http-equiv=\"refresh\" content=\"1; url=/wiki/İ\"></head></html>";
        assert_eq!(title_of(html), "İstanbul — Vikipedi");
        assert_eq!(meta_refresh(html, "https://tr.m.wikipedia.org/x").as_deref(), Some("https://tr.m.wikipedia.org/wiki/İ"));
    }

    #[test]
    fn challenge_detection() {
        let big = format!("<html><head><title>Tokyo</title><script>var x='Captcha';</script></head><body><p>{}</p></body></html>",
                          "Tokyo is the capital of Japan. ".repeat(200));
        assert!(!is_challenge(&big, &html_to_doc(&big, false).0));
        let bot = "<html><head><title>DuckDuckGo</title></head><body><p>Unfortunately, bots use DuckDuckGo too.</p></body></html>";
        assert!(is_challenge(bot, &html_to_doc(bot, false).0));
        let cf = "<html><head><title>Just a moment...</title></head><body></body></html>";
        assert!(is_challenge(cf, &html_to_doc(cf, false).0));
    }

    #[test]
    fn urls() {
        assert_eq!(url_join("https://en.m.wikipedia.org/wiki/Tokyo", "/wiki/Osaka"), "https://en.m.wikipedia.org/wiki/Osaka");
        assert_eq!(url_join("https://a.org/x/y.html", "../z.html"), "https://a.org/z.html");
        assert_eq!(url_join("https://a.org/x/", "//b.org/q"), "https://b.org/q");
        assert_eq!(url_join("https://h.com/", "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.org%2Fcaf%C3%A9&rut=1"),
                   "https://example.org/café");
        assert_eq!(percent_decode("x%zz%é"), "x%zz%é");
    }
}
