//! News: RSS/Atom feed reader. Articles open in the book reader; if a feed only has summaries,
//! the article page is fetched and shown in reading view. "Save for later" keeps a copy offline.

use crate::canvas::*;
use crate::epub::{html_to_doc, Book};
// parsing, discovery, dates and the catalogue are shared with the other platforms
pub use engine::feed::{discover_feed, host, is_feed, normalize_address, parse_feed, Feed, Item, CATALOG};
#[cfg(test)]
use engine::feed::{parse_date, resolve_url};
use crate::getbooks::http_get;
use crate::input::Gesture;
use crate::reader::Reader;
use crate::sys::Sys;
use crate::ui::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Feeds on a fresh install (until the user changes the list).
const DEFAULT_FEEDS: &[&str] = &[
    "https://www.thehindu.com/feeder/default.rss",
    "https://indianexpress.com/feed/",
    "https://timesofindia.indiatimes.com/rssfeedstopstories.cms",
    "https://feeds.feedburner.com/ndtvnews-top-stories",
    "https://www.livemint.com/rss/news",
    "https://www.livelaw.in/google_feeds.xml",
    "https://www.barandbench.com/stories.rss?time-period=last-24-hours",
    "https://www.espncricinfo.com/rss/content/story/feeds/0.xml",
    "https://feeds.bbci.co.uk/news/world/rss.xml",
];

const DEFAULTS_VERSION: &str = "2";

fn categories() -> Vec<&'static str> {
    let mut v: Vec<&str> = vec![];
    for c in CATALOG {
        if !v.contains(&c.0) {
            v.push(c.0);
        }
    }
    v
}

/// Browse suggested feeds by topic; one tap adds or removes a feed.
pub struct FeedCatalog {
    cat: usize,
    pager: Pager,
    urls: Vec<String>,
}

const C_CAT: u32 = 1_000_001;
const C_ADD_OWN: u32 = 1_000_002;
const TC_CAT: u32 = 1;
const TC_ADD: u32 = 2;

impl FeedCatalog {
    pub fn new(sys: &Sys) -> FeedCatalog {
        FeedCatalog { cat: 0, pager: Pager::new(78), urls: load_feed_urls(sys) }
    }
    fn items(&self) -> Vec<&'static (&'static str, &'static str, &'static str, &'static str)> {
        let c = categories()[self.cat];
        CATALOG.iter().filter(|x| x.0 == c).collect()
    }
}

impl App for FeedCatalog {
    fn title(&self) -> String {
        "Browse feeds".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let b = Btn::size(19);
        let cats = categories();
        cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x0 + 330, a.y0 + 58), &format!("{} ▾", cats[self.cat]), Some(Hit::App(C_CAT)), b);
        cx.button(Rect::new(a.x0 + 338, a.y0 + 10, a.x1 - 12, a.y0 + 58), "Other site…", Some(Hit::App(C_ADD_OWN)), b);
        let items = self.items();
        let urls = &self.urls;
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 70, a.x1, a.y1), items.len(), "", |cx, r, i| {
            let (_, name, desc, url) = items[i];
            let added = urls.iter().any(|u| u == url);
            let t = cx.ellipsize(name, Face::Bold, 21, r.w() - 170);
            cx.bold(r.x0 + 18, r.y0 + 14, &t, 21, BLACK, Anchor::LT);
            let d = cx.ellipsize(desc, Face::Sans, 15, r.w() - 170);
            cx.text(r.x0 + 18, r.y0 + 46, &d, 15, DARK, Anchor::LT);
            cx.hit(r, Hit::App(i as u32), None, name);
            cx.button(Rect::new(r.x1 - 140, r.cy() - 24, r.x1 - 14, r.cy() + 24), if added { "✓ Added" } else { "+ Add" },
                      Some(Hit::App(i as u32)), Btn { inverted: !added, ..Btn::size(18) });
        });
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        match id {
            C_CAT => Cmd::Dialog(Dialog::menu(TC_CAT, "Topic", categories().iter().map(|c| c.to_string()).collect())),
            C_ADD_OWN => Cmd::Dialog(Dialog::input(TC_ADD, "Add a site or feed", "", "Add")
                .with_message("Type a website, e.g. scroll.in. Folio finds its feed.")),
            i => {
                let Some(&(_, name, _, url)) = self.items().get(i as usize).copied() else { return Cmd::None };
                let msg = if let Some(p) = self.urls.iter().position(|u| u == url) {
                    self.urls.remove(p);
                    format!("Removed {name}")
                } else {
                    self.urls.push(url.to_string());
                    format!("Added {name}")
                };
                save_feed_urls(sys, &self.urls);
                Cmd::Batch(vec![Cmd::Fast, Cmd::Toast(msg)])
            }
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match tag {
            TC_CAT => {
                self.cat = choice.min(categories().len() - 1);
                self.pager = Pager::new(78);
                Cmd::Full
            }
            TC_ADD => match normalize_address(&text) {
                Some(u) => {
                    if !self.urls.contains(&u) {
                        self.urls.push(u);
                        save_feed_urls(sys, &self.urls);
                    }
                    // News finds the feed behind a website address when it refreshes
                    Cmd::Batch(vec![Cmd::Pop, Cmd::Toast("Added. Looking for its feed…".into())])
                }
                None => Cmd::Dialog(Dialog::message("Not an address", "Enter a feed or website address, e.g. scroll.in")),
            },
            _ => Cmd::None,
        }
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}

/// "thehindu.com" -> "https://thehindu.com"; None if it can't be an address.
/// Fetch and parse a feed. If the address is a normal web page (people often paste the site's
/// address), look for the feed it advertises (<link rel="alternate" type="application/rss+xml">).
pub fn fetch_feed(url: &str) -> Result<Feed, String> {
    let body = http_get(url)?;
    if is_feed(&body) {
        return Ok(parse_feed(url, &body));
    }
    let found = discover_feed(url, &body).ok_or_else(|| "This address isn't a feed, and the page doesn't link to one. \
        Look for an RSS link on the site.".to_string())?;
    let body = http_get(&found)?;
    if !is_feed(&body) {
        return Err("The page's feed link didn't return a feed.".into());
    }
    Ok(parse_feed(&found, &body))
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
                .args(["-sSfL", "-m", "20", "--max-filesize", "4000000", "-A", "Folio/0.2 (e-reader)", &u])
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
    let (mut blocks, links) = html_to_doc(&html, reading);
    if blocks.is_empty() {
        return Err("This article has no readable text.".into());
    }
    blocks.insert(0, crate::epub::Block { kind: crate::epub::BlockKind::Heading(1),
        runs: vec![crate::epub::Run { text: it.title.clone(), bold: true, italic: false, link: None }], start: 0 });
    let book = Book::from_blocks(&it.title, "", vec![(it.title.clone(), blocks)]).with_links(links);
    let mut r = Reader::with_book(book, &format!("news:{}", it.link), sys);
    if it.link.starts_with("http") {
        r.set_base_url(&it.link);
    }
    r.set_save_html(source_html);
    Ok(r)
}

enum Job {
    Feed(String, Result<Feed, String>),
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
    /// Headlines (all feeds merged) or the list of feeds; and the topic filter for Headlines.
    headlines_view: bool,
    topic: Option<&'static str>,
    last_partial_redraw: std::time::Instant,
}

const ID_TAB_HEADLINES: u32 = 1_000_010;
const ID_TAB_FEEDS: u32 = 1_000_011;
const ID_TOPIC: u32 = 1_000_100; // + topic index

/// Short names for the topic chips.
fn topic_label(cat: &str) -> &str {
    match cat {
        "Law & Opinion" => "Law",
        "Science & Tech" => "Tech",
        c => c,
    }
}

const ID_ADD: u32 = 1_000_001;
const ID_BACK: u32 = 1_000_002;
const ID_REFRESH: u32 = 1_000_003;
const ID_BROWSE: u32 = 1_000_004;
const T_ADD: u32 = 1;
const T_REMOVE: u32 = 2;
const FEED_ROW: i32 = 118;
const CARD_ROW: i32 = 128;

impl News {
    pub fn new(sys: &mut Sys) -> News {
        // once per new set of defaults: add them to an existing list (people who set up News before
        // the Indian feeds existed get them too; anything they removed later stays removed)
        let mut urls = load_feed_urls(sys);
        if sys.config.get("feed_defaults") != DEFAULTS_VERSION {
            // new defaults go first, so they're on the front page
            let mut merged: Vec<String> = DEFAULT_FEEDS.iter().filter(|d| !urls.iter().any(|u| u == *d)).map(|d| d.to_string()).collect();
            merged.extend(urls);
            urls = merged;
            save_feed_urls(sys, &urls);
            sys.config.set("feed_defaults", DEFAULTS_VERSION);
        }
        let headlines_view = sys.config.get("news_view") != "feeds";
        News { urls, feed: None, pager: Pager::new(if headlines_view { CARD_ROW } else { FEED_ROW }), job: None, busy_label: None, sel: 0,
               meta: load_meta(sys), loaded: HashMap::new(), refresh: None, refresh_left: 0, last_refresh: None,
               thumbs: Arc::new(Mutex::new(HashMap::new())), thumbs_running: Arc::new(AtomicBool::new(false)),
               thumbs_shown: 0, last_thumb_redraw: std::time::Instant::now(),
               headlines_view, topic: None, last_partial_redraw: std::time::Instant::now() }
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
        self.spawn("Loading feed…", move || {
            let r = fetch_feed(&url);
            Job::Feed(url, r)
        });
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
                let r = fetch_feed(&u);
                s2.lock().unwrap().push((u, r));
            }
        });
        self.refresh = Some(slot);
        self.last_refresh = Some(std::time::Instant::now());
    }

    /// Topic of a feed: its catalogue category, or "Other".
    fn feed_topic(url: &str) -> &'static str {
        CATALOG.iter().find(|c| c.3 == url).map_or("Other", |c| c.0)
    }

    /// Topics that the subscribed feeds cover, in catalogue order.
    fn topics(&self) -> Vec<&'static str> {
        let mut v: Vec<&'static str> = categories().into_iter()
            .filter(|c| self.urls.iter().any(|u| Self::feed_topic(u) == *c)).collect();
        if self.urls.iter().any(|u| Self::feed_topic(u) == "Other") {
            v.push("Other");
        }
        v
    }

    /// All loaded feeds merged: newest first, duplicates (same headline) dropped, optional topic.
    fn headlines(&self) -> Vec<(String, Item)> {
        let mut all: Vec<(String, Item)> = vec![];
        let mut seen = std::collections::HashSet::new();
        for u in &self.urls {
            if self.topic.map_or(false, |t| Self::feed_topic(u) != t) {
                continue;
            }
            let Some(f) = self.loaded.get(u) else { continue };
            let source = CATALOG.iter().find(|c| c.3 == u.as_str()).map(|c| c.1.to_string())
                .unwrap_or_else(|| f.title.chars().take(30).collect());
            for it in &f.items {
                let key: String = it.title.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
                if seen.insert(key) {
                    all.push((source.clone(), it.clone()));
                }
            }
        }
        all.sort_by_key(|(_, it)| std::cmp::Reverse(it.time().unwrap_or(0)));
        all.truncate(120);
        all
    }

    fn want_headline_thumbs(&mut self) {
        let urls: Vec<String> = self.headlines().iter().map(|(_, i)| i.thumb.clone()).filter(|t| !t.is_empty()).take(40).collect();
        if !urls.is_empty() && !self.thumbs_running.load(Ordering::SeqCst) {
            fetch_thumbs(urls, self.thumbs.clone(), self.thumbs_running.clone());
        }
    }

    fn render_headlines(&mut self, cx: &mut Ctx, a: Rect) {
        // topic chips
        let topics = self.topics();
        let mut chips: Vec<(Option<&'static str>, &str)> = vec![(None, "All")];
        chips.extend(topics.iter().map(|t| (Some(*t), topic_label(t))));
        let y = a.y0 + 62;
        let mut x = a.x0 + 12;
        let size = cx.sz(15);
        for (k, (t, label)) in chips.iter().enumerate() {
            let w = cx.f.width(label, Face::Sans, size) + 22;
            if x + w > a.x1 - 8 {
                break;
            }
            let on = *t == self.topic;
            cx.button(Rect::new(x, y, x + w, y + 40), label, Some(Hit::App(ID_TOPIC + k as u32)),
                      Btn { size: 15, inverted: on, radius: 18, ..Default::default() });
            x += w + 6;
        }
        let items = self.headlines();
        let refreshing = self.refresh.is_some();
        let empty = if refreshing { "Loading headlines…" } else if self.urls.is_empty() { "No feeds yet: tap Feeds → Browse feeds" }
                    else { "No stories yet" };
        if refreshing && !items.is_empty() {
            cx.text(a.x1 - 14, a.y0 + 112, "updating…", 12, DARK, Anchor::RT);
        }
        let area = Rect::new(a.x0, a.y0 + 110, a.x1, a.y1);
        let thumbs = self.thumbs.lock().unwrap();
        self.pager.render(cx, area, items.len(), empty, |cx, r, i| {
            let (src, it) = &items[i];
            let img = thumbs.get(&it.thumb).and_then(|g| g.as_ref());
            draw_story(cx, r, it, Some(src), img, i == 0);
            cx.hit(r, Hit::App(i as u32), None, &it.title);
        });
    }

    /// A site address turned out to have a feed elsewhere: remember the feed's address instead.
    fn adopt_url(&mut self, asked: &str, real: &str, sys: &Sys) {
        if asked != real {
            if let Some(u) = self.urls.iter_mut().find(|u| u.as_str() == asked) {
                *u = real.to_string();
                save_feed_urls(sys, &self.urls);
                self.meta.remove(asked);
            }
        }
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
            let tw = r.w() - 36 - 70; // room for the ⋯ button
            let ww = cx.sz(13) as i32 * when.chars().count() as i32 * 6 / 10 + 24;
            let t = cx.ellipsize(&name, Face::Bold, 21, tw - ww);
            cx.bold(r.x0 + 18, r.y0 + 12, &t, 21, BLACK, Anchor::LT);
            cx.text(r.x1 - 88, r.y0 + 16, &when, 13, DARK, Anchor::RT);
            let desc = m.map(|m| m.description.clone()).filter(|d| !d.is_empty()).unwrap_or_else(|| host(u));
            let d = cx.ellipsize(&desc, Face::Italic, 14, tw);
            cx.text_face(r.x0 + 18, r.y0 + 42, &d, Face::Italic, 14, DARK, Anchor::LT);
            match m {
                Some(m) if !m.headline.is_empty() => {
                    let h = cx.ellipsize(&format!("▸ {}", m.headline), Face::Sans, 17, tw);
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
            more_button(cx, r, 0x8000_0000 | i as u32);
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
            draw_story(cx, r, it, None, img, i == 0);
            cx.hit(r, Hit::App(i as u32), None, &it.title);
        });
    }
}

/// One story card: headline (2 lines), summary, "SOURCE · author · time · reading time", picture.
fn draw_story(cx: &mut Ctx, r: Rect, it: &Item, source: Option<&str>, img: Option<&crate::image::Gray>, lead: bool) {
    let tw = if img.is_some() { THUMB_W + 14 } else { 0 };
    let w = r.w() - 36 - tw;
    let (hs, ss) = (if lead { 22 } else { 20 }, 15u16);
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
        // always clip to the column: long words (URLs) don't wrap
    let l = if k + 1 == room && lines.len() > room { cx.ellipsize(&format!("{l}…"), Face::Sans, ss, w) } else { cx.ellipsize(l, Face::Sans, ss, w) };
        cx.text(r.x0 + 18, y + 2, &l, ss, DARK, Anchor::LT);
        y += sz as i32 + 4;
    }
    let _ = hl;
    let mut meta: Vec<String> = vec![];
    if let Some(src) = source {
        meta.push(src.to_uppercase());
    }
    if !it.author.is_empty() && source.is_none() {
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
}

impl App for News {
    fn title(&self) -> String {
        "News".into()
    }

    fn back(&mut self, _sys: &mut Sys) -> Option<Cmd> {
        let row = if self.headlines_view { CARD_ROW } else { FEED_ROW };
        self.feed.take().map(|_| {
            self.pager = Pager::new(row);
            Cmd::Full
        })
    }

    fn on_show(&mut self, sys: &mut Sys) {
        // the feed list may have changed in "Browse feeds"
        let urls = load_feed_urls(sys);
        if urls != self.urls {
            self.urls = urls;
            self.last_refresh = None;
        }
        let stale = self.last_refresh.map_or(true, |t| t.elapsed().as_secs() > 15 * 60);
        if self.feed.is_none() && stale {
            self.refresh_all();
        }
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let b = Btn::size(19);
        if self.feed.is_some() {
            cx.button(Rect::new(a.x1 - 170, a.y0 + 8, a.x1 - 12, a.y0 + 56), "Refresh", Some(Hit::App(ID_REFRESH)), b);
            self.render_feed(cx, a);
        } else {
            let hv = self.headlines_view;
            cx.button(Rect::new(a.x0 + 12, a.y0 + 8, a.cx() - 4, a.y0 + 54), "Headlines", Some(Hit::App(ID_TAB_HEADLINES)),
                      Btn { inverted: hv, bold: hv, ..b });
            cx.button(Rect::new(a.cx() + 4, a.y0 + 8, a.x1 - 12, a.y0 + 54), "Feeds", Some(Hit::App(ID_TAB_FEEDS)),
                      Btn { inverted: !hv, bold: !hv, ..b });
            if hv {
                if let Some(l) = &self.busy_label {
                    cx.text(a.cx(), a.y1 - 16, l, 15, DARK, Anchor::MM);
                }
                self.render_headlines(cx, a);
            } else {
                cx.button(Rect::new(a.x0 + 12, a.y0 + 62, a.cx() - 4, a.y0 + 106), "Browse feeds", Some(Hit::App(ID_BROWSE)), b);
                cx.button(Rect::new(a.cx() + 4, a.y0 + 62, a.x1 - 12, a.y0 + 106), "+ Add site", Some(Hit::App(ID_ADD)), b);
                let label = self.busy_label.clone().or_else(|| self.refresh.is_some().then(|| "Checking feeds…".to_string()));
                if let Some(l) = label {
                    cx.text(a.cx(), a.y0 + 124, &l, 15, DARK, Anchor::MM);
                }
                self.render_front(cx, Rect::new(a.x0, a.y0 + 136, a.x1, a.y1));
            }
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
                        self.adopt_url(&u, &f.url, sys);
                        self.meta.insert(f.url.clone(), meta_of(&f));
                        self.loaded.insert(f.url.clone(), f);
                    }
                    Err(_) => self.meta.entry(u).or_default().error = true,
                }
            }
            if self.refresh_left == 0 {
                self.refresh = None;
                save_meta(sys, &self.meta);
                redraw = self.feed.is_none();
                if self.headlines_view && self.feed.is_none() {
                    self.want_headline_thumbs();
                }
            } else if self.headlines_view && self.feed.is_none() && self.last_partial_redraw.elapsed().as_secs() >= 4 {
                // show stories as feeds arrive, without flashing the screen for each one
                self.last_partial_redraw = std::time::Instant::now();
                redraw = true;
            }
        }
        // thumbnails: redraw when new ones arrived (at most every 2 s, and once when done)
        if self.feed.is_some() || self.headlines_view {
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
            Job::Feed(asked, Ok(f)) => {
                self.adopt_url(&asked, &f.url, sys);
                self.meta.insert(f.url.clone(), meta_of(&f));
                save_meta(sys, &self.meta);
                self.loaded.insert(f.url.clone(), f.clone());
                self.show_feed(f);
                Cmd::Full
            }
            Job::Feed(_, Err(e)) => Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Couldn't load feed", &e))]),
            Job::Article(it, page) => match article_reader(&it, page, sys) {
                Ok(r) => Cmd::Push(Box::new(r)),
                Err(e) => Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Couldn't open article", &e))]),
            },
        }
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        if self.job.is_some() {
            return Cmd::Toast("Still loading…".into());
        }
        match id {
            ID_TAB_HEADLINES | ID_TAB_FEEDS => {
                self.headlines_view = id == ID_TAB_HEADLINES;
                sys.config.set("news_view", if self.headlines_view { "headlines" } else { "feeds" });
                self.pager = Pager::new(if self.headlines_view { CARD_ROW } else { FEED_ROW });
                if self.headlines_view {
                    self.want_headline_thumbs();
                }
                Cmd::Full
            }
            t if (ID_TOPIC..ID_TOPIC + 20).contains(&t) && self.feed.is_none() && self.headlines_view => {
                let k = (t - ID_TOPIC) as usize;
                self.topic = if k == 0 { None } else { self.topics().get(k - 1).copied() };
                self.pager = Pager::new(CARD_ROW);
                self.want_headline_thumbs();
                Cmd::Full
            }
            ID_BROWSE => Cmd::Push(Box::new(FeedCatalog::new(sys))),
            ID_ADD => Cmd::Dialog(Dialog::input(T_ADD, "Add a site or feed", "", "Add").with_message("Just the website works, e.g. thehindu.com")),
            ID_BACK => {
                self.feed = None;
                self.pager = Pager::new(if self.headlines_view { CARD_ROW } else { FEED_ROW });
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
                if self.headlines_view {
                    if let Some((_, it)) = self.headlines().get(i as usize).cloned() {
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
                let Some(u) = normalize_address(&text) else {
                    return Cmd::Dialog(Dialog::message("Not an address", "Enter a feed or website address, e.g. thehindu.com"));
                };
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
            <media:content url="http://e/1.jpg" medium="image"><media:title>Caption</media:title><media:credit>Photo</media:credit></media:content></item>
            <item><title>Two</title><author>mail@e.org (B. Writer)</author><description>&lt;img src="http://e/2.png"&gt;Hi</description></item>
            </channel></rss>"#;
        let f = parse_feed("http://e/feed", xml);
        assert_eq!(f.items[0].title, "One");
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
        assert_eq!(f.items.len(), 1);
        assert_eq!((f.description.as_str(), f.items[0].author.as_str(), f.items[0].summary.as_str()), ("Notes", "C. Writer", "Short"));
    }

    #[test]
    fn discovery() {
        let html = r#"<html><head><link rel="alternate" type="application/atom+xml" href="/atom.xml">
            <link rel="alternate" type="application/rss+xml" title="News" href="/google_feeds.xml"></head><body></body></html>"#;
        assert_eq!(discover_feed("https://www.example.in/", html).as_deref(), Some("https://www.example.in/google_feeds.xml"));
        assert_eq!(resolve_url("https://a.org/x/y.html", "feed.xml"), "https://a.org/x/feed.xml");
        assert_eq!(resolve_url("https://a.org/x/", "//cdn.b.org/f"), "https://cdn.b.org/f");
        assert!(is_feed("<?xml version=\"1.0\"?><rss version=\"2.0\">"));
        assert!(!is_feed("<!DOCTYPE html><html>"));
    }
}
