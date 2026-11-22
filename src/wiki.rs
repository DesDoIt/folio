//! Wikipedia: search, read articles in the book reader (sections become chapters), and keep a copy
//! of every opened article for offline reading.

use crate::canvas::*;
use crate::epub::{html_to_doc, Block, BlockKind, Book};
use crate::getbooks::http_get;
use crate::input::Gesture;
use crate::json;
use crate::reader::Reader;
use crate::sys::Sys;
use crate::ui::*;
use std::sync::{Arc, Mutex};

const LANGS: [&str; 8] = ["en", "hi", "de", "fr", "es", "ja", "it", "pt"];

fn enc(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'(' | b')' => (b as char).to_string(),
        b' ' => "_".into(),
        _ => format!("%{b:02X}"),
    }).collect()
}

fn cache_dir(sys: &Sys) -> String {
    format!("{}/kos/wikipedia", sys.us)
}

fn cache_file(sys: &Sys, lang: &str, title: &str) -> String {
    let safe: String = title.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c }).take(100).collect();
    format!("{}/{lang} - {safe}.html", cache_dir(sys))
}

/// Article HTML -> chapters (one per h2 section; the intro is "Introduction").
fn split_sections(title: &str, html: &str) -> Vec<(String, Vec<Block>)> {
    let (blocks, _) = html_to_doc(html, false);
    let mut chapters: Vec<(String, Vec<Block>)> = vec![("Introduction".into(), vec![Block {
        kind: BlockKind::Heading(1),
        runs: vec![crate::epub::Run { text: title.into(), bold: true, italic: false, link: None }], start: 0 }])];
    for b in blocks {
        let text: String = b.runs.iter().map(|r| r.text.as_str()).collect();
        // drop edit links and reference clutter
        if text == "[edit]" || text.starts_with("Jump to") {
            continue;
        }
        if b.kind == BlockKind::Heading(2) {
            let t = text.trim_end_matches("[edit]").trim().to_string();
            if matches!(t.as_str(), "References" | "External links" | "Notes" | "Citations" | "Sources" | "Further reading") {
                break; // skip the reference sections at the end
            }
            chapters.push((t, vec![b]));
        } else {
            chapters.last_mut().unwrap().1.push(b);
        }
    }
    // re-number character offsets per chapter so saved positions work
    for (_, blocks) in chapters.iter_mut() {
        let mut off = 0;
        for b in blocks.iter_mut() {
            b.start = off;
            off += b.chars();
        }
    }
    chapters
}

fn fetch_article(lang: &str, title: &str) -> Result<String, String> {
    http_get(&format!("https://{lang}.wikipedia.org/w/index.php?title={}&action=render", enc(title)))
}

fn search(lang: &str, q: &str) -> Result<Vec<String>, String> {
    let body = http_get(&format!("https://{lang}.wikipedia.org/w/api.php?action=opensearch&limit=12&namespace=0&format=json&search={}",
                                 enc(q).replace('_', "%20")))?;
    let j = json::parse(&body).map_err(|e| e.to_string())?;
    Ok(j.arr().get(1).map(|a| a.arr().iter().filter_map(|t| t.str().map(String::from)).collect()).unwrap_or_default())
}

fn random_title(lang: &str) -> Result<String, String> {
    let body = http_get(&format!("https://{lang}.wikipedia.org/w/api.php?action=query&list=random&rnnamespace=0&rnlimit=1&format=json"))?;
    let j = json::parse(&body).map_err(|e| e.to_string())?;
    j.get("query").get("random").arr().first().and_then(|r| r.get("title").str()).map(String::from)
        .ok_or_else(|| "No random article".into())
}

enum Job {
    Results(Result<Vec<String>, String>),
    Article(String, Result<String, String>),
}

pub struct Wiki {
    lang: String,
    query: Option<String>,
    results: Vec<String>,
    saved: Vec<String>,
    pager: Pager,
    job: Option<Arc<Mutex<Option<Job>>>>,
    busy: Option<String>,
}

const ID_SEARCH: u32 = 1_000_001;
const ID_RANDOM: u32 = 1_000_002;
const ID_LANG: u32 = 1_000_003;
const ID_SAVED: u32 = 1_000_004;
const T_SEARCH: u32 = 1;
const T_LANG: u32 = 2;

impl Wiki {
    pub fn new(sys: &Sys) -> Wiki {
        let lang = Some(sys.config.get("wiki_lang").to_string()).filter(|l| !l.is_empty()).unwrap_or_else(|| "en".into());
        Wiki { lang, query: None, results: vec![], saved: vec![], pager: Pager::new(62), job: None, busy: None }
    }

    fn spawn(&mut self, label: &str, f: impl FnOnce() -> Job + Send + 'static) {
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            let r = f();
            *s2.lock().unwrap() = Some(r);
        });
        self.job = Some(slot);
        self.busy = Some(label.into());
    }

    fn load_saved(&mut self, sys: &Sys) {
        let mut v: Vec<(i64, String)> = std::fs::read_dir(cache_dir(sys)).map(|rd| rd.flatten().filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let t = e.metadata().ok().and_then(|m| m.modified().ok())
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            n.strip_suffix(".html").map(|s| (t, s.to_string()))
        }).collect()).unwrap_or_default();
        v.sort_by_key(|x| -x.0);
        self.saved = v.into_iter().map(|x| x.1).collect();
    }

    fn open(&mut self, title: String, sys: &Sys) -> Cmd {
        // offline copy first
        let cached = cache_file(sys, &self.lang, &title);
        if let Ok(html) = std::fs::read_to_string(&cached) {
            return self.reader(&title, &html, sys);
        }
        let lang = self.lang.clone();
        self.spawn("Loading article…", move || {
            let r = fetch_article(&lang, &title);
            Job::Article(title, r)
        });
        Cmd::Redraw
    }

    fn reader(&self, title: &str, html: &str, sys: &Sys) -> Cmd {
        let chapters = split_sections(title, html);
        let book = Book::from_blocks(title, "Wikipedia", chapters);
        Cmd::Push(Box::new(Reader::with_book(book, &format!("wiki:{}:{title}", self.lang), sys)))
    }
}

impl App for Wiki {
    fn title(&self) -> String {
        format!("Wikipedia ({})", self.lang)
    }

    fn on_show(&mut self, sys: &mut Sys) {
        self.load_saved(sys);
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let b = Btn::size(19);
        cx.button(Rect::new(a.x0 + 10, a.y0 + 10, a.x0 + 230, a.y0 + 58), "Search", Some(Hit::App(ID_SEARCH)), Btn { inverted: true, ..b });
        cx.button(Rect::new(a.x0 + 238, a.y0 + 10, a.x0 + 388, a.y0 + 58), "Random", Some(Hit::App(ID_RANDOM)), b);
        cx.button(Rect::new(a.x0 + 396, a.y0 + 10, a.x0 + 496, a.y0 + 58), "Saved", Some(Hit::App(ID_SAVED)),
                  Btn { inverted: self.query.is_none(), ..b });
        cx.button(Rect::new(a.x0 + 504, a.y0 + 10, a.x1 - 10, a.y0 + 58), &self.lang.clone(), Some(Hit::App(ID_LANG)), b);
        let heading = match &self.query {
            Some(q) => format!("Results for “{q}”"),
            None => format!("Saved for offline reading ({})", self.saved.len()),
        };
        cx.bold(a.x0 + 16, a.y0 + 72, &heading, 18, BLACK, Anchor::LT);
        if let Some(l) = &self.busy {
            cx.text(a.x1 - 16, a.y0 + 72, l, 16, DARK, Anchor::RT);
        }
        let list: &Vec<String> = if self.query.is_some() { &self.results } else { &self.saved };
        let empty = if self.query.is_some() { "No results" } else { "Articles you open are kept here" };
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 102, a.x1, a.y1), list.len(), empty, |cx, r, i| {
            let t = cx.ellipsize(&list[i], Face::Sans, 20, r.w() - 40);
            cx.text(r.x0 + 20, r.cy(), &t, 20, BLACK, Anchor::LM);
            cx.hit(r, Hit::App(i as u32), None, &list[i]);
        });
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn poll(&mut self, sys: &mut Sys) -> Cmd {
        let done = self.job.as_ref().and_then(|j| j.lock().unwrap().take());
        let Some(done) = done else { return Cmd::None };
        self.job = None;
        self.busy = None;
        match done {
            Job::Results(Ok(r)) => {
                self.results = r;
                self.pager.page = 0;
                Cmd::Full
            }
            Job::Results(Err(e)) | Job::Article(_, Err(e)) => Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Wikipedia", &e))]),
            Job::Article(title, Ok(html)) => {
                let _ = std::fs::create_dir_all(cache_dir(sys));
                let _ = std::fs::write(cache_file(sys, &self.lang, &title), &html);
                self.load_saved(sys);
                self.reader(&title, &html, sys)
            }
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
            ID_SEARCH => Cmd::Dialog(Dialog::input(T_SEARCH, "Search Wikipedia", "", "Search")),
            ID_RANDOM => {
                let lang = self.lang.clone();
                self.spawn("Picking an article…", move || match random_title(&lang) {
                    Ok(t) => {
                        let r = fetch_article(&lang, &t);
                        Job::Article(t, r)
                    }
                    Err(e) => Job::Results(Err(e)),
                });
                Cmd::Redraw
            }
            ID_SAVED => {
                self.query = None;
                self.load_saved(sys);
                self.pager.page = 0;
                Cmd::Full
            }
            ID_LANG => Cmd::Dialog(Dialog::menu(T_LANG, "Wikipedia language", LANGS.iter().map(|l| l.to_string()).collect())),
            i => {
                if self.query.is_some() {
                    match self.results.get(i as usize).cloned() {
                        Some(t) => self.open(t, sys),
                        None => Cmd::None,
                    }
                } else {
                    // saved entries are "<lang> - <title>"
                    match self.saved.get(i as usize).cloned() {
                        Some(n) => {
                            let (lang, title) = n.split_once(" - ").map(|(l, t)| (l.to_string(), t.to_string()))
                                .unwrap_or((self.lang.clone(), n));
                            let path = cache_file(sys, &lang, &title);
                            match std::fs::read_to_string(&path) {
                                Ok(html) => {
                                    let prev = std::mem::replace(&mut self.lang, lang);
                                    let c = self.reader(&title, &html, sys);
                                    self.lang = prev;
                                    c
                                }
                                Err(e) => Cmd::Toast(e.to_string()),
                            }
                        }
                        None => Cmd::None,
                    }
                }
            }
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match tag {
            T_SEARCH if !text.trim().is_empty() => {
                let (lang, q) = (self.lang.clone(), text.trim().to_string());
                self.query = Some(q.clone());
                self.results.clear();
                self.spawn("Searching…", move || Job::Results(search(&lang, &q)));
                Cmd::Redraw
            }
            T_LANG => {
                if let Some(l) = LANGS.get(choice) {
                    self.lang = l.to_string();
                    sys.config.set("wiki_lang", l);
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
