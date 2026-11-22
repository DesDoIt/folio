//! News: RSS/Atom feed reader. Articles open in the book reader; if a feed only has summaries,
//! the article page is fetched and shown in reading view. "Save for later" keeps a copy offline.

use crate::canvas::*;
use crate::epub::{decode_entities, html_to_doc, tokenize, Book, Tok};
use crate::getbooks::http_get;
use crate::input::Gesture;
use crate::reader::Reader;
use crate::sys::Sys;
use crate::ui::*;
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
}

#[derive(Clone)]
pub struct Feed {
    pub url: String,
    pub title: String,
    pub items: Vec<Item>,
}

/// Parse RSS 2.0 or Atom.
pub fn parse_feed(url: &str, xml: &str) -> Feed {
    let mut feed = Feed { url: url.into(), title: String::new(), items: vec![] };
    let mut cur: Option<Item> = None;
    let mut field: Option<String> = None;
    let mut text = String::new();
    for t in tokenize(xml) {
        match t {
            Tok::Open(n, a, sc) => match n.as_str() {
                "item" | "entry" => cur = Some(Item::default()),
                "title" | "pubdate" | "updated" | "published" | "description" | "summary" | "encoded" | "content" => {
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
                if field.as_deref() == Some(n.as_str()) {
                    let v = text.trim().to_string();
                    field = None;
                    match cur.as_mut() {
                        Some(it) => match n.as_str() {
                            "title" => it.title = decode_entities(&v),
                            "link" => it.link = v,
                            "pubdate" | "updated" | "published" if it.date.is_empty() => it.date = v,
                            "encoded" | "content" => it.content = decode_entities(&v),
                            "description" | "summary" if it.content.is_empty() => it.content = decode_entities(&v),
                            _ => {}
                        },
                        None if n == "title" && feed.title.is_empty() => feed.title = decode_entities(&v),
                        _ => {}
                    }
                } else if n == "item" || n == "entry" {
                    if let Some(it) = cur.take() {
                        if !it.title.is_empty() {
                            feed.items.push(it);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if feed.title.is_empty() {
        feed.title = url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url).to_string();
    }
    feed
}

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

/// Short, human date from RSS/Atom formats ("Sat, 21 Nov 2026 10:00:00 GMT" / "2026-11-21T10:00:00Z").
fn short_date(d: &str) -> String {
    if d.len() >= 10 && d.as_bytes()[4] == b'-' {
        return d[..10].to_string();
    }
    d.split_whitespace().skip(1).take(3).collect::<Vec<_>>().join(" ")
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
}

const ID_ADD: u32 = 1_000_001;
const ID_BACK: u32 = 1_000_002;
const ID_REFRESH: u32 = 1_000_003;
const T_ADD: u32 = 1;
const T_REMOVE: u32 = 2;

impl News {
    pub fn new(sys: &Sys) -> News {
        News { urls: load_feed_urls(sys), feed: None, pager: Pager::new(70), job: None, busy_label: None, sel: 0 }
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
}

impl App for News {
    fn title(&self) -> String {
        self.feed.as_ref().map_or("News".into(), |f| f.title.clone())
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let b = Btn::size(19);
        if self.feed.is_some() {
            cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x0 + 190, a.y0 + 58), "◀ All feeds", Some(Hit::App(ID_BACK)), b);
            cx.button(Rect::new(a.x1 - 170, a.y0 + 10, a.x1 - 12, a.y0 + 58), "Refresh", Some(Hit::App(ID_REFRESH)), b);
        } else {
            cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x1 - 12, a.y0 + 58), "+ Add feed", Some(Hit::App(ID_ADD)),
                      Btn { inverted: true, ..b });
        }
        if let Some(l) = &self.busy_label {
            cx.text(a.cx(), a.y0 + 80, l, 18, DARK, Anchor::MM);
        }
        let area = Rect::new(a.x0, a.y0 + 100, a.x1, a.y1);
        match &self.feed {
            Some(f) => {
                let items = &f.items;
                self.pager.render(cx, area, items.len(), "No articles", |cx, r, i| {
                    let it = &items[i];
                    let t = cx.ellipsize(&it.title, Face::Bold, 19, r.w() - 40);
                    cx.bold(r.x0 + 18, r.y0 + 12, &t, 19, BLACK, Anchor::LT);
                    cx.text(r.x0 + 18, r.y0 + 42, &short_date(&it.date), 15, DARK, Anchor::LT);
                    cx.hit(r, Hit::App(i as u32), None, &it.title);
                });
            }
            None => {
                let urls = &self.urls;
                self.pager.render(cx, area, urls.len(), "No feeds — add one", |cx, r, i| {
                    let host = urls[i].split("://").nth(1).unwrap_or(&urls[i]).to_string();
                    let t = cx.ellipsize(&host, Face::Bold, 19, r.w() - 40);
                    cx.bold(r.x0 + 18, r.y0 + 14, &t, 19, BLACK, Anchor::LT);
                    cx.text(r.x0 + 18, r.y0 + 44, "Tap to read · hold to remove", 14, DARK, Anchor::LT);
                    cx.hit(r, Hit::App(i as u32), Some(Hit::App(0x8000_0000 | i as u32)), &host);
                });
            }
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn poll(&mut self, sys: &mut Sys) -> Cmd {
        let done = self.job.as_ref().and_then(|j| j.lock().unwrap().take());
        let Some(done) = done else { return Cmd::None };
        self.job = None;
        self.busy_label = None;
        match done {
            Job::Feed(Ok(f)) => {
                self.feed = Some(f);
                self.pager.page = 0;
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
                self.pager.page = 0;
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
                    Some(u) => {
                        self.open_feed(u);
                        Cmd::Redraw
                    }
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
                self.urls.push(u);
                save_feed_urls(sys, &self.urls);
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

