//! News: RSS/Atom feed reader. Articles open in the book reader; if a feed only has summaries,
//! the article page is fetched and shown in reading view. "Save for later" keeps a copy offline.

use crate::canvas::*;
use crate::epub::{decode_entities, html_to_doc, tokenize, Book, Tok};
use crate::getbooks::http_get;
use crate::input::Gesture;
use crate::reader::Reader;
use crate::sys::Sys;
use crate::ui::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const DEFAULT_FEEDS: &[&str] = &[
    "https://feeds.bbci.co.uk/news/world/rss.xml",
    "https://hnrss.org/frontpage",
    "https://www.theguardian.com/books/rss",
    "https://standardebooks.org/feeds/atom/new-releases",
];

#[derive(Clone, Default)]
pub struct Item {
    pub title: String,
    pub link: String,
    pub date: String,
    pub content: String, // HTML (full text or summary)
    pub summary: String, // plain text teaser
    pub author: String,
    pub thumb: String, // image URL, if the feed has one
}

#[derive(Clone, Default)]
pub struct Feed {
    pub url: String,
    pub title: String,
    pub description: String,
    pub items: Vec<Item>,
}

impl Item {
    pub fn time(&self) -> Option<i64> {
        parse_date(&self.date)
    }
    /// Estimated reading time, only when the feed carries the full article.
    pub fn minutes(&self) -> Option<usize> {
        let words = plain_text(&self.content).split_whitespace().count();
        (words >= 300).then(|| (words + 229) / 230)
    }
}

fn plain_text(html: &str) -> String {
    let (blocks, _) = html_to_doc(html, false);
    let mut out = String::new();
    for b in blocks.iter() {
        for r in b.runs.iter() {
            out.push_str(&r.text);
        }
        out.push(' ');
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_img(html: &str) -> Option<String> {
    tokenize(html).into_iter().find_map(|t| match t {
        Tok::Open(n, a, _) if n == "img" => a.get("src").filter(|s| s.starts_with("http") || s.starts_with("file:")).cloned(),
        _ => None,
    })
}

/// Parse RSS 2.0 or Atom, with the usual extensions (dc:creator, media:thumbnail/content, enclosures).
pub fn parse_feed(url: &str, xml: &str) -> Feed {
    let mut feed = Feed { url: url.into(), ..Default::default() };
    let mut cur: Option<Item> = None;
    let mut field: Option<String> = None;
    let mut text = String::new();
    let mut in_author = false;
    let mut in_image = false; // the channel's <image> block has its own <title>
    for t in tokenize(xml) {
        match t {
            Tok::Open(n, a, sc) => match n.as_str() {
                "item" | "entry" => cur = Some(Item::default()),
                "image" if cur.is_none() && !sc => in_image = true,
                "author" if !sc => {
                    in_author = true;
                    field = Some("author".into());
                    text.clear();
                }
                "name" if in_author => {
                    field = Some("name".into());
                    text.clear();
                }
                "thumbnail" | "content" | "enclosure" if a.contains_key("url") => {
                    // media:thumbnail / media:content / <enclosure>: keep the first image
                    let is_img = n == "thumbnail"
                        || a.get("medium").map_or(false, |m| m == "image")
                        || a.get("type").map_or(n == "content", |t| t.starts_with("image/"));
                    if let (Some(it), true) = (cur.as_mut(), is_img) {
                        if it.thumb.is_empty() {
                            it.thumb = a["url"].clone();
                        }
                    }
                }
                "title" | "pubdate" | "updated" | "published" | "date" | "description" | "summary" | "encoded"
                | "content" | "creator" | "subtitle" => {
                    if !sc {
                        field = Some(n.clone());
                        text.clear();
                    }
                }
                "link" => {
                    let href = a.get("href").cloned();
                    let rel = a.get("rel").cloned().unwrap_or_else(|| "alternate".into());
                    match (cur.as_mut(), href) {
                        (Some(it), Some(h)) if rel == "alternate" && it.link.is_empty() => it.link = h,
                        (Some(it), Some(h)) if rel == "enclosure" && it.thumb.is_empty()
                            && a.get("type").map_or(false, |t| t.starts_with("image/")) => it.thumb = h,
                        (Some(_), None) if !sc => {
                            field = Some("link".into());
                            text.clear();
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            Tok::Text(x) if field.is_some() => text.push_str(x),
            Tok::Close(n) => {
                if n == "image" {
                    in_image = false;
                }
                if field.as_deref() == Some(n.as_str()) {
                    let v = text.trim().to_string();
                    field = None;
                    match cur.as_mut() {
                        Some(it) => match n.as_str() {
                            "title" => it.title = decode_entities(&v),
                            "link" => it.link = v,
                            "pubdate" | "updated" | "published" | "date" if it.date.is_empty() => it.date = v,
                            "encoded" | "content" => it.content = decode_entities(&v),
                            "description" | "summary" => {
                                let html = decode_entities(&v);
                                if it.summary.is_empty() {
                                    it.summary = plain_text(&html);
                                }
                                if it.content.is_empty() {
                                    it.content = html;
                                }
                            }
                            "creator" | "name" | "author" if it.author.is_empty() => {
                                // RSS <author> is often "mail@example.org (Name)"
                                let v = decode_entities(&v);
                                let v = match (v.find('('), v.rfind(')')) {
                                    (Some(a), Some(b)) if a < b && v.contains('@') => v[a + 1..b].to_string(),
                                    _ => v,
                                };
                                it.author = v.trim().to_string();
                            }
                            _ => {}
                        },
                        None if in_image => {}
                        None if n == "title" && feed.title.is_empty() => feed.title = decode_entities(&v),
                        None if (n == "description" || n == "subtitle") && feed.description.is_empty() => {
                            feed.description = plain_text(&decode_entities(&v))
                        }
                        _ => {}
                    }
                    if n == "name" {
                        field = Some("author".into()); // still inside <author>
                        text.clear();
                    }
                } else if n == "author" {
                    in_author = false;
                    field = None;
                } else if n == "item" || n == "entry" {
                    in_author = false;
                    if let Some(mut it) = cur.take() {
                        if it.summary.is_empty() {
                            it.summary = plain_text(&it.content);
                        }
                        if it.summary.chars().count() > 400 {
                            it.summary = it.summary.chars().take(400).collect();
                        }
                        if it.thumb.is_empty() {
                            it.thumb = first_img(&it.content).unwrap_or_default();
                        }
                        if !it.title.is_empty() {
                            feed.items.push(it);
                        }
                    }
                }
                if n == "author" {
                    in_author = false;
                }
            }
            _ => {}
        }
    }
    if feed.title.is_empty() {
        feed.title = host(url);
    }
    feed
}

fn host(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    if url.starts_with("file:") {
        return rest.rsplit('/').next().unwrap_or(rest).to_string();
    }
    rest.split('/').next().unwrap_or(rest).trim_start_matches("www.").to_string()
}

// --- Dates -----------------------------------------------------------------------------------------
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// RFC 822 ("Sat, 21 Nov 2026 09:00:00 +0100") or ISO 8601 ("2026-11-21T09:00:00Z") to a Unix time.
pub fn parse_date(d: &str) -> Option<i64> {
    let d = d.trim();
    let num = |s: &str| s.parse::<i64>().ok();
    let tz = |z: &str| -> i64 {
        let z = z.trim();
        if let Some(sign) = z.chars().next().filter(|c| *c == '+' || *c == '-') {
            let digits: String = z[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 4 {
                let off = num(&digits[..2]).unwrap_or(0) * 3600 + num(&digits[2..4]).unwrap_or(0) * 60;
                return if sign == '+' { -off } else { off };
            }
        }
        match z {
            "EST" => 5 * 3600, "EDT" => 4 * 3600, "CST" => 6 * 3600, "CDT" => 5 * 3600,
            "MST" => 7 * 3600, "MDT" => 6 * 3600, "PST" => 8 * 3600, "PDT" => 7 * 3600,
            _ => 0,
        }
    };
    let hms = |t: &str| -> Option<i64> {
        let mut p = t.split(':');
        Some(num(p.next()?)? * 3600 + num(p.next()?)? * 60 + p.next().and_then(|s| num(s.get(..2).unwrap_or(s))).unwrap_or(0))
    };
    if d.len() >= 10 && d.as_bytes().get(4) == Some(&b'-') {
        let day = days_from_civil(num(&d[0..4])?, num(&d[5..7])?, num(&d[8..10])?);
        let rest = d.get(11..).unwrap_or("");
        let (time, zone) = match rest.find(|c| c == 'Z' || c == '+' || c == '-') {
            Some(p) => (&rest[..p], &rest[p..]),
            None => (rest, ""),
        };
        let secs = if time.is_empty() { 0 } else { hms(time.split('.').next().unwrap_or(time))? };
        return Some(day * 86400 + secs + if zone == "Z" { 0 } else { tz(zone) });
    }
    let parts: Vec<&str> = d.split_whitespace().filter(|p| !p.ends_with(',')).collect();
    let (dd, mon, yy, time) = (num(parts.first()?)?, parts.get(1)?, num(parts.get(2)?)?, parts.get(3)?);
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let m = MONTHS.iter().position(|x| mon.to_ascii_lowercase().starts_with(x))? as i64 + 1;
    let yy = if yy < 100 { yy + 2000 } else { yy };
    Some(days_from_civil(yy, m, dd) * 86400 + hms(time)? + tz(parts.get(4).unwrap_or(&"")))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// "just now", "12 min ago", "3 h ago", "yesterday", "21 Nov".
pub fn relative(t: i64) -> String {
    let ago = now() - t;
    match ago {
        i64::MIN..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", ago / 60),
        3600..=86399 => format!("{} h ago", ago / 3600),
        86400..=172799 => "yesterday".into(),
        _ => crate::sys::format_time(t, "%d %b"),
    }
}

/// What the front page shows for each feed (cached, so it appears before the refresh finishes).
#[derive(Clone, Default)]
struct Meta {
    title: String,
    description: String,
    count: usize,
    newest: i64,
    headline: String,
    fetched: i64,
    error: bool,
}

fn meta_path(sys: &Sys) -> String {
    format!("{}/kos/news-cache.txt", sys.us)
}

fn load_meta(sys: &Sys) -> BTreeMap<String, Meta> {
    let mut m = BTreeMap::new();
    for l in std::fs::read_to_string(meta_path(sys)).unwrap_or_default().lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() >= 7 {
            m.insert(f[0].to_string(), Meta { title: f[1].into(), description: f[2].into(), count: f[3].parse().unwrap_or(0),
                newest: f[4].parse().unwrap_or(0), headline: f[5].into(), fetched: f[6].parse().unwrap_or(0), error: false });
        }
    }
    m
}

fn save_meta(sys: &Sys, m: &BTreeMap<String, Meta>) {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let body: String = m.iter().map(|(u, x)| format!("{u}\t{}\t{}\t{}\t{}\t{}\t{}\n", clean(&x.title),
        clean(&x.description.chars().take(200).collect::<String>()), x.count, x.newest, clean(&x.headline), x.fetched)).collect();
    let _ = std::fs::write(meta_path(sys), body);
}

fn meta_of(f: &Feed) -> Meta {
    let newest = f.items.iter().filter_map(|i| i.time().map(|t| (t, i))).max_by_key(|(t, _)| *t);
    Meta {
        title: f.title.clone(),
        description: f.description.clone(),
        count: f.items.len(),
        newest: newest.map_or(0, |(t, _)| t),
        headline: newest.map(|(_, i)| i.title.clone()).or_else(|| f.items.first().map(|i| i.title.clone())).unwrap_or_default(),
        fetched: now(),
        error: false,
    }
}

/// Download small grey thumbnails for a feed's articles (background thread).
fn fetch_thumbs(urls: Vec<String>, store: Thumbs, running: Arc<AtomicBool>) {
    running.store(true, Ordering::SeqCst);
    std::thread::spawn(move || {
        for u in urls {
            if store.lock().unwrap().contains_key(&u) {
                continue;
            }
            let img = std::process::Command::new("curl")
                .args(["-sSfL", "-m", "20", "--max-filesize", "4000000", "-A", "KindleLinux/0.2 (e-reader)", &u])
                .stdin(std::process::Stdio::null()).output().ok()
                .filter(|o| o.status.success())
                .and_then(|o| crate::image::decode(&o.stdout).ok())
                .map(|g| crate::image::fill(&g, THUMB_W, THUMB_H));
            store.lock().unwrap().insert(u, img);
        }
        running.store(false, Ordering::SeqCst);
    });
}

const THUMB_W: i32 = 120;
const THUMB_H: i32 = 90;
type Thumbs = Arc<Mutex<HashMap<String, Option<crate::image::Gray>>>>;

fn feeds_path(sys: &Sys) -> String {
    format!("{}/kos/feeds.txt", sys.us)
}

fn load_feed_urls(sys: &Sys) -> Vec<String> {
    match std::fs::read_to_string(feeds_path(sys)) {
        Ok(s) => s.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect(),
        Err(_) => DEFAULT_FEEDS.iter().map(|s| s.to_string()).collect(),
    }
}

fn save_feed_urls(sys: &Sys, urls: &[String]) {
    let _ = std::fs::create_dir_all(format!("{}/kos", sys.us));
    let _ = std::fs::write(feeds_path(sys), urls.join("\n") + "\n");
}

/// Download step (background thread): the article page, if the feed only carries a summary.
fn fetch_article(it: &Item) -> Option<String> {
    let plain_len = html_to_doc(&it.content, false).0.iter().map(|b| b.chars()).sum::<usize>();
    if plain_len < 1500 && it.link.starts_with("http") { http_get(&it.link).ok() } else { None }
}

/// Build a reader for an article: full feed text if substantial, else the fetched page (reading view).
pub fn article_reader(it: &Item, page: Option<String>, sys: &Sys) -> Result<Reader, String> {
    let (html, reading, source_html) = match page {
        Some(p) => (p.clone(), true, p),
        None => (it.content.clone(), false,
                 format!("<html><head><title>{}</title></head><body>{}</body></html>", it.title, it.content)),
    };
    let (mut blocks, _) = html_to_doc(&html, reading);
    if blocks.is_empty() {
        return Err("This article has no readable text.".into());
    }
    blocks.insert(0, crate::epub::Block { kind: crate::epub::BlockKind::Heading(1),
        runs: vec![crate::epub::Run { text: it.title.clone(), bold: true, italic: false, link: None }], start: 0 });
    let book = Book::from_blocks(&it.title, "", vec![(it.title.clone(), blocks)]);
    let mut r = Reader::with_book(book, &format!("news:{}", it.link), sys);
    r.set_save_html(source_html);
    Ok(r)
}

enum Job {
    Feed(Result<Feed, String>),
    Article(Item, Option<String>),
}

pub struct News {
    urls: Vec<String>,
    feed: Option<Feed>,
    pager: Pager,
    job: Option<Arc<Mutex<Option<Job>>>>,
    busy_label: Option<String>,
    sel: usize,
    /// Front page: cached details per feed, and feeds fetched this session.
    meta: BTreeMap<String, Meta>,
    loaded: HashMap<String, Feed>,
    refresh: Option<Arc<Mutex<Vec<(String, Result<Feed, String>)>>>>,
    refresh_left: usize,
    last_refresh: Option<std::time::Instant>,
    thumbs: Thumbs,
    thumbs_running: Arc<AtomicBool>,
    thumbs_shown: usize,
    last_thumb_redraw: std::time::Instant,
}

const ID_ADD: u32 = 1_000_001;
const ID_BACK: u32 = 1_000_002;
const ID_REFRESH: u32 = 1_000_003;
const T_ADD: u32 = 1;
const T_REMOVE: u32 = 2;
const FEED_ROW: i32 = 118;
const CARD_ROW: i32 = 128;

impl News {
    pub fn new(sys: &Sys) -> News {
        News { urls: load_feed_urls(sys), feed: None, pager: Pager::new(FEED_ROW), job: None, busy_label: None, sel: 0,
               meta: load_meta(sys), loaded: HashMap::new(), refresh: None, refresh_left: 0, last_refresh: None,
               thumbs: Arc::new(Mutex::new(HashMap::new())), thumbs_running: Arc::new(AtomicBool::new(false)),
               thumbs_shown: 0, last_thumb_redraw: std::time::Instant::now() }
    }

    fn spawn(&mut self, label: &str, f: impl FnOnce() -> Job + Send + 'static) {
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            let r = f();
            *s2.lock().unwrap() = Some(r);
        });
        self.job = Some(slot);
        self.busy_label = Some(label.into());
    }

    fn open_feed(&mut self, url: String) {
        self.spawn("Loading feed…", move || Job::Feed(http_get(&url).map(|x| parse_feed(&url, &x))));
    }

    /// Refresh every feed in the background for the front page.
    fn refresh_all(&mut self) {
        if self.refresh.is_some() || self.urls.is_empty() {
            return;
        }
        let slot = Arc::new(Mutex::new(vec![]));
        let (s2, urls) = (slot.clone(), self.urls.clone());
        self.refresh_left = urls.len();
        std::thread::spawn(move || {
            for u in urls {
                let r = http_get(&u).map(|x| parse_feed(&u, &x));
                s2.lock().unwrap().push((u, r));
            }
        });
        self.refresh = Some(slot);
        self.last_refresh = Some(std::time::Instant::now());
    }

    fn show_feed(&mut self, f: Feed) {
        let thumbs: Vec<String> = f.items.iter().map(|i| i.thumb.clone()).filter(|t| !t.is_empty()).take(40).collect();
        if !thumbs.is_empty() {
            fetch_thumbs(thumbs, self.thumbs.clone(), self.thumbs_running.clone());
        }
        self.feed = Some(f);
        self.pager = Pager::new(CARD_ROW);
    }

    fn render_front(&mut self, cx: &mut Ctx, area: Rect) {
        let (urls, meta) = (&self.urls, &self.meta);
        let refreshing = self.refresh.is_some();
        self.pager.render(cx, area, urls.len(), "No feeds — add one", |cx, r, i| {
            let u = &urls[i];
            let m = meta.get(u);
            let name = m.map(|m| m.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| host(u));
            let when = m.filter(|m| m.newest > 0).map(|m| relative(m.newest)).unwrap_or_default();
            let ww = cx.sz(13) as i32 * when.chars().count() as i32 * 6 / 10 + 24;
            let t = cx.ellipsize(&name, Face::Bold, 21, r.w() - 36 - ww);
            cx.bold(r.x0 + 18, r.y0 + 12, &t, 21, BLACK, Anchor::LT);
            cx.text(r.x1 - 18, r.y0 + 16, &when, 13, DARK, Anchor::RT);
            let desc = m.map(|m| m.description.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| host(u));
            let d = cx.ellipsize(&desc, Face::Italic, 14, r.w() - 36);
            cx.text_face(r.x0 + 18, r.y0 + 42, &d, Face::Italic, 14, DARK, Anchor::LT);
            match m {
                Some(m) if !m.headline.is_empty() => {
                    let h = cx.ellipsize(&format!("▸ {}", m.headline), Face::Sans, 17, r.w() - 36);
                    cx.text(r.x0 + 18, r.y0 + 64, &h, 17, BLACK, Anchor::LT);
                    let state = if m.error { " · couldn't refresh".to_string() } else { String::new() };
                    cx.text(r.x0 + 18, r.y0 + 92, &format!("{} stories{state}", m.count), 13, DARK, Anchor::LT);
                }
                _ => {
                    let s = if refreshing { "Loading…" } else { "Tap to read" };
                    cx.text(r.x0 + 18, r.y0 + 66, s, 16, DARK, Anchor::LT);
                }
            }
            cx.hit(r, Hit::App(i as u32), Some(Hit::App(0x8000_0000 | i as u32)), &name);
        });
    }

    fn render_feed(&mut self, cx: &mut Ctx, a: Rect) {
        let Some(f) = &self.feed else { return };
        // masthead
        let (x0, x1) = (a.x0 + 16, a.x1 - 16);
        cx.hline(x0, x1, a.y0 + 66, 3, BLACK);
        let t = cx.ellipsize(&f.title, Face::Bold, 28, x1 - x0);
        cx.bold(a.cx(), a.y0 + 74, &t, 28, BLACK, Anchor::MT);
        if !f.description.is_empty() {
            let d = cx.ellipsize(&f.description, Face::Italic, 14, x1 - x0);
            cx.text_face(a.cx(), a.y0 + 112, &d, Face::Italic, 14, DARK, Anchor::MT);
        }
        let newest = f.items.iter().filter_map(|i| i.time()).max();
        let line = match (&self.busy_label, newest) {
            (Some(l), _) => l.clone(),
            (None, Some(t)) => format!("UPDATED {} · {} STORIES", relative(t).to_uppercase(), f.items.len()),
            (None, None) => format!("{} STORIES", f.items.len()),
        };
        cx.text(a.cx(), a.y0 + 132, &line, 12, DARK, Anchor::MT);
        cx.hline(x0, x1, a.y0 + 152, 1, BLACK);
        let area = Rect::new(a.x0, a.y0 + 156, a.x1, a.y1);
        let items = &f.items;
        let thumbs = self.thumbs.lock().unwrap();
        self.pager.render(cx, area, items.len(), "No articles", |cx, r, i| {
            let it = &items[i];
            let img = thumbs.get(&it.thumb).and_then(|g| g.as_ref());
            let tw = if img.is_some() { THUMB_W + 14 } else { 0 };
            let w = r.w() - 36 - tw;
            let (hs, ss) = (if i == 0 { 22 } else { 20 }, 15u16);
            let (hz, sz) = (cx.sz(hs), cx.sz(ss));
            let mut y = r.y0 + 10;
            let head = cx.f.wrap(&it.title, Face::Bold, hz, w);
            let hl = head.len().min(2);
            for (k, l) in head.iter().take(2).enumerate() {
                let l = if k == 1 && head.len() > 2 { cx.ellipsize(&format!("{l}…"), Face::Bold, hs, w) } else { l.clone() };
                cx.bold(r.x0 + 18, y, &l, hs, BLACK, Anchor::LT);
                y += hz as i32 + 5;
            }
            let room = ((r.y1 - 30 - y) / (sz as i32 + 4)).clamp(0, 3) as usize;
            let summary = if it.summary.starts_with(&it.title) { "" } else { it.summary.as_str() };
            let lines = cx.f.wrap(summary, Face::Sans, sz, w);
            for (k, l) in lines.iter().take(room).enumerate() {
                let l = if k + 1 == room && lines.len() > room { cx.ellipsize(&format!("{l}…"), Face::Sans, ss, w) } else { l.clone() };
                cx.text(r.x0 + 18, y + 2, &l, ss, DARK, Anchor::LT);
                y += sz as i32 + 4;
            }
            let _ = hl;
            let mut meta: Vec<String> = vec![];
            if !it.author.is_empty() {
                meta.push(it.author.clone());
            }
            if let Some(t) = it.time() {
                meta.push(relative(t));
            }
            if let Some(m) = it.minutes() {
                meta.push(format!("{m} min read"));
            }
            let m = cx.ellipsize(&meta.join("  ·  "), Face::Bold, 12, w);
            cx.bold(r.x0 + 18, r.y1 - 24, &m, 12, DARK, Anchor::LT);
            if let Some(g) = img {
                let (tx, ty) = (r.x1 - 18 - THUMB_W, r.y0 + (r.h() - THUMB_H) / 2);
                cx.c.blit(&g.px, g.w, g.h, tx, ty);
                for edge in [Rect::new(tx - 1, ty - 1, tx + THUMB_W + 1, ty), Rect::new(tx - 1, ty + THUMB_H, tx + THUMB_W + 1, ty + THUMB_H + 1),
                             Rect::new(tx - 1, ty, tx, ty + THUMB_H), Rect::new(tx + THUMB_W, ty, tx + THUMB_W + 1, ty + THUMB_H)] {
                    cx.c.fill(edge, BLACK);
                }
            }
            cx.hit(r, Hit::App(i as u32), None, &it.title);
        });
    }
}

impl App for News {
    fn title(&self) -> String {
        "News".into()
    }

    fn on_show(&mut self, _sys: &mut Sys) {
        let stale = self.last_refresh.map_or(true, |t| t.elapsed().as_secs() > 15 * 60);
        if self.feed.is_none() && stale {
            self.refresh_all();
        }
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let b = Btn::size(19);
        if self.feed.is_some() {
            cx.button(Rect::new(a.x0 + 12, a.y0 + 8, a.x0 + 190, a.y0 + 56), "◀ All feeds", Some(Hit::App(ID_BACK)), b);
            cx.button(Rect::new(a.x1 - 170, a.y0 + 8, a.x1 - 12, a.y0 + 56), "Refresh", Some(Hit::App(ID_REFRESH)), b);
            self.render_feed(cx, a);
        } else {
            cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x1 - 12, a.y0 + 58), "+ Add feed", Some(Hit::App(ID_ADD)),
                      Btn { inverted: true, ..b });
            let label = self.busy_label.clone().or_else(|| self.refresh.is_some().then(|| "Checking feeds…".to_string()));
            if let Some(l) = label {
                cx.text(a.cx(), a.y0 + 80, &l, 16, DARK, Anchor::MM);
            }
            self.render_front(cx, Rect::new(a.x0, a.y0 + 100, a.x1, a.y1));
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.refresh.is_some() || self.thumbs_running.load(Ordering::SeqCst)
    }

    fn poll(&mut self, sys: &mut Sys) -> Cmd {
        // front-page refresh results
        let mut redraw = false;
        if let Some(slot) = &self.refresh {
            let got: Vec<_> = slot.lock().unwrap().drain(..).collect();
            for (u, r) in got {
                self.refresh_left = self.refresh_left.saturating_sub(1);
                match r {
                    Ok(f) => {
                        self.meta.insert(u.clone(), meta_of(&f));
                        self.loaded.insert(u, f);
                    }
                    Err(_) => self.meta.entry(u).or_default().error = true,
                }
            }
            if self.refresh_left == 0 {
                self.refresh = None;
                save_meta(sys, &self.meta);
                redraw = self.feed.is_none();
            }
        }
        // thumbnails: redraw when new ones arrived (at most every 2 s, and once when done)
        if self.feed.is_some() {
            let n = self.thumbs.lock().unwrap().values().filter(|v| v.is_some()).count();
            let running = self.thumbs_running.load(Ordering::SeqCst);
            if n != self.thumbs_shown && (!running || self.last_thumb_redraw.elapsed().as_millis() >= 2000) {
                self.thumbs_shown = n;
                self.last_thumb_redraw = std::time::Instant::now();
                redraw = true;
            }
        }
        let done = self.job.as_ref().and_then(|j| j.lock().unwrap().take());
        let Some(done) = done else { return if redraw { Cmd::Redraw } else { Cmd::None } };
        self.job = None;
        self.busy_label = None;
        match done {
            Job::Feed(Ok(f)) => {
                self.meta.insert(f.url.clone(), meta_of(&f));
                save_meta(sys, &self.meta);
                self.loaded.insert(f.url.clone(), f.clone());
                self.show_feed(f);
                Cmd::Full
            }
            Job::Feed(Err(e)) => Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Couldn't load feed", &e))]),
            Job::Article(it, page) => match article_reader(&it, page, sys) {
                Ok(r) => Cmd::Push(Box::new(r)),
                Err(e) => Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Couldn't open article", &e))]),
            },
        }
    }

    fn action(&mut self, id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        if self.job.is_some() {
            return Cmd::Toast("Still loading…".into());
        }
        match id {
            ID_ADD => Cmd::Dialog(Dialog::input(T_ADD, "Add feed", "https://", "Add").with_message("RSS or Atom feed address")),
            ID_BACK => {
                self.feed = None;
                self.pager = Pager::new(FEED_ROW);
                Cmd::Full
            }
            ID_REFRESH => {
                if let Some(f) = &self.feed {
                    let u = f.url.clone();
                    self.open_feed(u);
                }
                Cmd::Redraw
            }
            i if i & 0x8000_0000 != 0 => {
                self.sel = (i & 0x7fff_ffff) as usize;
                let u = self.urls.get(self.sel).cloned().unwrap_or_default();
                Cmd::Dialog(Dialog::confirm(T_REMOVE, "Remove feed?", &u, "Remove"))
            }
            i => {
                if let Some(f) = &self.feed {
                    if let Some(it) = f.items.get(i as usize).cloned() {
                        self.spawn("Opening article…", move || {
                            let page = fetch_article(&it);
                            Job::Article(it, page)
                        });
                    }
                    return Cmd::Redraw;
                }
                match self.urls.get(i as usize).cloned() {
                    Some(u) => match self.loaded.get(&u).cloned() {
                        Some(f) => {
                            self.show_feed(f);
                            Cmd::Full
                        }
                        None => {
                            self.open_feed(u);
                            Cmd::Redraw
                        }
                    },
                    None => Cmd::None,
                }
            }
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match tag {
            T_ADD => {
                let u = text.trim().to_string();
                if !u.starts_with("http") {
                    return Cmd::Dialog(Dialog::message("Not a feed address", "Enter a full http(s):// address."));
                }
                self.urls.push(u.clone());
                save_feed_urls(sys, &self.urls);
                self.open_feed(u);
                Cmd::Redraw
            }
            T_REMOVE => {
                if self.sel < self.urls.len() {
                    self.urls.remove(self.sel);
                    save_feed_urls(sys, &self.urls);
                }
                Cmd::Redraw
            }
            _ => Cmd::None,
        }
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(parse_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse_date("1970-01-02T00:00:00Z"), Some(86400));
        assert_eq!(parse_date("Sat, 21 Nov 2026 10:00:00 +0100"), parse_date("2026-11-21T09:00:00Z"));
        assert_eq!(parse_date("2026-11-21T14:30:00+05:30"), parse_date("2026-11-21T09:00:00.000Z"));
        assert_eq!(parse_date("nonsense"), None);
    }

    #[test]
    fn feed_metadata() {
        let xml = r#"<rss xmlns:dc="x" xmlns:media="y"><channel><title>Gazette</title><description>Daily &amp; local</description>
            <image><title>logo</title><url>http://e/l.png</url></image>
            <item><title>One</title><dc:creator>A. Writer</dc:creator><description>&lt;p&gt;Teaser &lt;b&gt;text&lt;/b&gt;&lt;/p&gt;</description>
            <media:content url="http://e/1.jpg" medium="image"><media:credit>Photo</media:credit></media:content></item>
            <item><title>Two</title><author>mail@e.org (B. Writer)</author><description>&lt;img src="http://e/2.png"&gt;Hi</description></item>
            </channel></rss>"#;
        let f = parse_feed("http://e/feed", xml);
        assert_eq!(f.title, "Gazette");
        assert_eq!(f.description, "Daily & local");
        assert_eq!(f.items[0].author, "A. Writer");
        assert_eq!(f.items[0].summary, "Teaser text");
        assert_eq!(f.items[0].thumb, "http://e/1.jpg");
        assert_eq!(f.items[1].author, "B. Writer");
        assert_eq!(f.items[1].thumb, "http://e/2.png");
        let atom = r#"<feed><title>Blog</title><subtitle>Notes</subtitle><entry><title>E</title><author><name>C. Writer</name></author>
            <updated>2026-11-21T09:00:00Z</updated><summary>Short</summary></entry></feed>"#;
        let f = parse_feed("http://b/atom", atom);
        assert_eq!((f.description.as_str(), f.items[0].author.as_str(), f.items[0].summary.as_str()), ("Notes", "C. Writer", "Short"));
    }
}
