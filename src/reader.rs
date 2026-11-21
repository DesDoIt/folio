//! Simple built-in book reader (EPUB, TXT): full-screen pages, tap/swipe to turn,
//! top-middle tap for a menu. Positions are saved per book as (chapter, character offset),
//! so they survive font-size changes.

use crate::apps::open_koreader;
use crate::canvas::*;
use crate::epub::{Block, BlockKind, Book};
use crate::input::{Gesture, Kind};
use crate::sys::Sys;
use crate::ui::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const MARGIN_X: i32 = 36;
const MARGIN_TOP: i32 = 34;
const FOOTER_H: i32 = 40;
pub const FONT_SIZES: [u16; 6] = [16, 18, 20, 22, 24, 28];

struct Word {
    text: String,
    face: Face,
    w: i32,
    offset: usize,
    link: Option<u32>,
}

pub struct Line {
    words: Vec<Word>,
    y: i32,
    x0: i32,
    avail: i32,
    size: u16,
    justify: bool,
    center: bool,
    rule: bool,
}

pub struct Page {
    lines: Vec<Line>,
    pub start: usize,
}

struct Style {
    size: u16,
    face: Face,
    center: bool,
    indent: i32,
    left: i32,
    before: i32,
    after: i32,
    justify: bool,
    prefix: &'static str,
}

fn style(kind: BlockKind, fs: u16) -> Style {
    let f = fs as i32;
    let base = Style { size: fs, face: Face::Sans, center: false, indent: f * 3 / 2, left: 0, before: 0,
                       after: f * 2 / 5, justify: true, prefix: "" };
    match kind {
        BlockKind::Para => base,
        BlockKind::Heading(l) => {
            let size = match l { 1 => fs * 3 / 2, 2 => fs * 13 / 10, _ => fs * 11 / 10 };
            Style { size, face: Face::Bold, center: l <= 2, indent: 0, before: f, after: f * 3 / 5, justify: false, ..base }
        }
        BlockKind::Item => Style { indent: 0, left: f, justify: false, prefix: "• ", after: f / 4, ..base },
        BlockKind::Quote => Style { left: f * 3 / 2, indent: 0, size: fs * 19 / 20, ..base },
        BlockKind::Pre => Style { face: Face::Mono, size: fs * 17 / 20, indent: 0, justify: false, ..base },
        BlockKind::Rule => Style { indent: 0, before: f / 2, after: f / 2, justify: false, ..base },
    }
}

/// Lay out a chapter's blocks into pages of the given area.
pub fn paginate(blocks: &[Block], fonts: &mut Fonts, fs: u16, area: Rect) -> Vec<Page> {
    let mut pages = vec![Page { lines: vec![], start: 0 }];
    let top = area.y0 + MARGIN_TOP;
    let bottom = area.y1 - FOOTER_H;
    let width = area.w() - 2 * MARGIN_X;
    let mut y = top;
    for (bi, b) in blocks.iter().enumerate() {
        let st = style(b.kind, fs);
        let line_h = st.size as i32 * 145 / 100;
        if y > top {
            y += st.before;
        }
        if b.kind == BlockKind::Rule {
            if y + line_h > bottom {
                pages.push(Page { lines: vec![], start: b.start });
                y = top;
            }
            pages.last_mut().unwrap().lines.push(Line { words: vec![], y, x0: area.x0 + MARGIN_X, avail: width,
                size: st.size, justify: false, center: false, rule: true });
            y += line_h;
            continue;
        }
        // headings: don't strand them at the bottom of a page
        if matches!(b.kind, BlockKind::Heading(_)) && y > top && y + line_h * 3 > bottom {
            pages.push(Page { lines: vec![], start: b.start });
            y = top;
        }
        // words
        let mut words: Vec<Word> = vec![];
        let mut off = b.start;
        if !st.prefix.is_empty() {
            words.push(Word { text: st.prefix.trim_end().into(), face: st.face, w: fonts.width(st.prefix.trim_end(), st.face, st.size), offset: off, link: None });
        }
        for run in &b.runs {
            let face = if st.face == Face::Mono { Face::Mono } else if run.bold { Face::Bold } else { st.face };
            for (i, w) in run.text.split(' ').enumerate() {
                if i > 0 {
                    off += 1;
                }
                if w.is_empty() {
                    continue;
                }
                let ww = fonts.width(w, face, st.size);
                words.push(Word { text: w.to_string(), face, w: ww, offset: off, link: run.link });
                off += w.chars().count();
            }
        }
        let space = fonts.width(" ", st.face, st.size).max(3);
        // list items: wrapped lines hang under the text, not the bullet
        let hang = if st.prefix.is_empty() { 0 } else { fonts.width(st.prefix, st.face, st.size) };
        let mut i = 0;
        let mut first = true;
        while i < words.len() {
            let indent = if first && st.indent > 0 && bi > 0 { st.indent } else if !first { hang } else { 0 };
            let x0 = area.x0 + MARGIN_X + st.left + indent;
            let avail = width - st.left - indent;
            let mut line_words = vec![];
            let mut used = 0;
            while i < words.len() {
                let w = &words[i];
                let need = if line_words.is_empty() { w.w } else { used + space + w.w };
                if need <= avail {
                    used = need;
                    line_words.push(std::mem::replace(&mut words[i], Word { text: String::new(), face: st.face, w: 0, offset: 0, link: None }));
                    i += 1;
                } else if line_words.is_empty() {
                    // a single word wider than the line: split it by characters
                    let chars: Vec<char> = w.text.chars().collect();
                    let mut cut = chars.len();
                    while cut > 1 && fonts.width(&chars[..cut].iter().collect::<String>(), w.face, st.size) > avail {
                        cut -= 1;
                    }
                    let head: String = chars[..cut].iter().collect();
                    let tail: String = chars[cut..].iter().collect();
                    let (face, offset, link) = (w.face, w.offset, w.link);
                    let hw = fonts.width(&head, face, st.size);
                    let tw = fonts.width(&tail, face, st.size);
                    line_words.push(Word { text: head, face, w: hw, offset, link });
                    words[i] = Word { text: tail, face, w: tw, offset: offset + cut, link };
                    break;
                } else {
                    break;
                }
            }
            if y + line_h > bottom {
                let start = line_words.first().map_or(b.start, |w| w.offset);
                pages.push(Page { lines: vec![], start });
                y = top;
            }
            let last_line = i >= words.len();
            pages.last_mut().unwrap().lines.push(Line { words: line_words, y, x0, avail, size: st.size,
                justify: st.justify && !last_line, center: st.center, rule: false });
            y += line_h;
            first = false;
        }
        y += st.after;
    }
    pages.retain(|p| !p.lines.is_empty());
    if pages.is_empty() {
        pages.push(Page { lines: vec![], start: 0 });
    }
    pages
}

/// Where things ended up on screen after drawing a page.
#[derive(Default)]
pub struct PageGeom {
    pub links: Vec<(Rect, u32)>,
    pub words: Vec<(Rect, usize, String)>, // (box, chapter offset, text)
}

/// Draw a page; returns link areas (rect, link index) and word boxes.
pub fn draw_page(cx: &mut Ctx, page: &Page) -> PageGeom {
    let mut links: Vec<(Rect, u32)> = vec![];
    let mut words: Vec<(Rect, usize, String)> = vec![];
    for line in &page.lines {
        if line.rule {
            let w = line.avail / 4;
            let x = line.x0 + (line.avail - w) / 2;
            cx.c.hline(x, x + w, line.y + line.size as i32 / 2, 1, DARK);
            continue;
        }
        let n = line.words.len() as i32;
        if n == 0 {
            continue;
        }
        let space = cx.f.width(" ", Face::Sans, line.size).max(3);
        let total: i32 = line.words.iter().map(|w| w.w).sum::<i32>() + space * (n - 1);
        let mut gap = space as f32;
        let mut x = line.x0 as f32;
        if line.center {
            x = (line.x0 + (line.avail - total) / 2) as f32;
        } else if line.justify && n > 1 {
            let g = (line.avail - (total - space * (n - 1))) as f32 / (n - 1) as f32;
            // justify unless the gaps would be absurd (very few words on a wide line)
            if g <= space as f32 * 4.0 || line.size >= 24 && g <= space as f32 * 6.0 {
                gap = g;
            }
        }
        let lh = line.size as i32 * 145 / 100;
        for w in &line.words {
            let wx = x.round() as i32;
            cx.c.text(cx.f, wx, line.y, &w.text, w.face, line.size, BLACK, Anchor::LT);
            words.push((Rect::new(wx - 2, line.y - 4, wx + w.w + 2, line.y + lh - 2), w.offset, w.text.clone()));
            if let Some(l) = w.link {
                let uy = line.y + line.size as i32 * 118 / 100;
                cx.c.hline(wx, wx + w.w, uy, 1, BLACK);
                let r = Rect::new(wx - 3, line.y - 4, wx + w.w + 3, line.y + lh - 2);
                // merge with the previous word of the same link on this line
                match links.last_mut() {
                    Some((pr, pl)) if *pl == l && pr.y0 == r.y0 && r.x0 - pr.x1 < 40 => pr.x1 = r.x1,
                    _ => links.push((r, l)),
                }
            }
            x += w.w as f32 + gap;
        }
    }
    PageGeom { links, words }
}

// --- saved positions ---------------------------------------------------------------------------
fn positions_path(sys: &Sys) -> String {
    format!("{}/kos/reading.txt", sys.us)
}

fn load_positions(sys: &Sys) -> HashMap<String, (usize, usize)> {
    let mut m = HashMap::new();
    for line in std::fs::read_to_string(positions_path(sys)).unwrap_or_default().lines() {
        let p: Vec<&str> = line.split('\t').collect();
        if p.len() >= 3 {
            if let (Ok(c), Ok(o)) = (p[1].parse(), p[2].parse()) {
                m.insert(p[0].to_string(), (c, o));
            }
        }
    }
    m
}

fn save_position(sys: &Sys, path: &str, chapter: usize, offset: usize) {
    let mut m = load_positions(sys);
    m.insert(path.to_string(), (chapter, offset));
    let body: String = m.iter().map(|(k, (c, o))| format!("{k}\t{c}\t{o}\n")).collect();
    let _ = std::fs::create_dir_all(format!("{}/kos", sys.us));
    let _ = std::fs::write(positions_path(sys), body);
}

// --- Reader app ---------------------------------------------------------------------------------
pub struct Reader {
    path: String,
    book: Book,
    chapter: usize,
    pages: Vec<Page>,
    page: usize,
    fs: u16,
    laid_out: Option<(usize, u16, Rect)>,
    want_offset: Option<usize>,
    want_last: bool,
    pick: Arc<Mutex<Option<usize>>>,
    /// HTML to keep if the user picks "Save for later" (articles, web pages)
    save_html: Option<String>,
    words: Vec<(Rect, usize, String)>,
    sel_word: Option<(usize, String)>,
    define_job: Option<Arc<Mutex<Option<Result<String, String>>>>>,
    last_turn: std::time::Instant,
}

const ID_WORD: u32 = 4;
const TAG_WORD: u32 = 3;
const ID_PREV: u32 = 1;
const ID_NEXT: u32 = 2;
const ID_MENU: u32 = 3;
const TAG_MENU: u32 = 1;
const TAG_FONT: u32 = 2;

impl Reader {
    pub fn open(path: &str, sys: &Sys) -> Result<Reader, String> {
        Ok(Reader::with_book(Book::open(path)?, path, sys))
    }

    /// Read an already-loaded book; `key` identifies it for saved positions and stats.
    pub fn with_book(book: Book, key: &str, sys: &Sys) -> Reader {
        let fs = sys.config.get("reader_font").parse().unwrap_or(20);
        let (chapter, offset) = load_positions(sys).get(key).copied().unwrap_or((0, 0));
        let chapter = chapter.min(book.spine.len().saturating_sub(1));
        Reader { path: key.into(), book, chapter, pages: vec![], page: 0, fs, laid_out: None,
                 want_offset: Some(offset), want_last: false, pick: Arc::new(Mutex::new(None)), save_html: None,
                 words: vec![], sel_word: None, define_job: None, last_turn: std::time::Instant::now() }
    }

    pub fn set_save_html(&mut self, html: String) {
        self.save_html = Some(html);
    }

    /// Count reading time between page turns (capped, so a book left open doesn't count).
    fn track(&mut self, sys: &Sys, pages: u64) {
        let secs = self.last_turn.elapsed().as_secs().min(300);
        self.last_turn = std::time::Instant::now();
        crate::highlights::record(sys, secs, pages);
    }

    /// The sentence around a chapter offset.
    fn sentence_at(&mut self, off: usize) -> String {
        let blocks = self.book.chapter_blocks(self.chapter);
        let Some(b) = blocks.iter().rev().find(|b| b.start <= off) else { return String::new() };
        let text: String = b.runs.iter().map(|r| r.text.as_str()).collect();
        let chars: Vec<char> = text.chars().collect();
        let i = (off - b.start).min(chars.len().saturating_sub(1));
        let is_end = |c: char| matches!(c, '.' | '!' | '?' | '。' | '！' | '？');
        let mut s = i;
        while s > 0 && !is_end(chars[s - 1]) {
            s -= 1;
        }
        let mut e = i;
        while e < chars.len() && !is_end(chars[e]) {
            e += 1;
        }
        let e = (e + 1).min(chars.len());
        chars[s..e].iter().collect::<String>().trim().to_string()
    }

    fn start_offset(&self) -> usize {
        self.pages.get(self.page).map_or(0, |p| p.start)
    }

    fn goto_chapter(&mut self, c: usize, last: bool) {
        self.chapter = c;
        self.laid_out = None;
        self.want_offset = None;
        self.want_last = last;
        self.page = 0;
    }

    fn next(&mut self, sys: &Sys) -> Cmd {
        if self.page + 1 < self.pages.len() {
            self.page += 1;
        } else if self.chapter + 1 < self.book.spine.len() {
            self.goto_chapter(self.chapter + 1, false);
        } else {
            if !self.path.starts_with("news:") && !self.path.starts_with("wiki:") {
                crate::highlights::mark_finished(sys, &self.book.title);
            }
            return Cmd::Toast("End of book".into());
        }
        self.track(sys, 1);
        self.save(sys);
        Cmd::Redraw
    }

    fn prev(&mut self, sys: &Sys) -> Cmd {
        if self.page > 0 {
            self.page -= 1;
        } else if self.chapter > 0 {
            self.goto_chapter(self.chapter - 1, true);
        } else {
            return Cmd::None;
        }
        self.save(sys);
        Cmd::Redraw
    }

    fn save(&self, sys: &Sys) {
        // when a chapter change is pending, its start is the position
        let off = if self.laid_out.is_none() { 0 } else { self.start_offset() };
        save_position(sys, &self.path, self.chapter, off);
    }

    fn progress(&self) -> u32 {
        let n = self.book.spine.len().max(1) as f32;
        let within = if self.pages.len() > 1 { self.page as f32 / self.pages.len() as f32 } else { 0.0 };
        (((self.chapter as f32 + within) / n) * 100.0) as u32
    }
}

impl App for Reader {
    fn title(&self) -> String {
        self.book.title.clone()
    }

    fn fullscreen(&self) -> bool {
        true
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        if self.laid_out != Some((self.chapter, self.fs, a)) {
            let blocks = self.book.chapter_blocks(self.chapter);
            self.pages = paginate(&blocks, cx.f, self.fs, a);
            self.page = if self.want_last {
                self.pages.len() - 1
            } else if let Some(off) = self.want_offset.take() {
                self.pages.iter().rposition(|p| p.start <= off).unwrap_or(0)
            } else {
                self.page.min(self.pages.len() - 1)
            };
            self.want_last = false;
            self.laid_out = Some((self.chapter, self.fs, a));
        }
        if let Some(p) = self.pages.get(self.page) {
            self.words = draw_page(cx, p).words;
        }
        // footer: chapter title, page within chapter, book progress
        let fy = a.y1 - FOOTER_H / 2 - 2;
        cx.c.hline(a.x0 + MARGIN_X, a.x1 - MARGIN_X, a.y1 - FOOTER_H + 4, 1, LIGHT);
        let right = format!("{} / {}  ·  {}%", self.page + 1, self.pages.len(), self.progress());
        let rw = cx.text(a.x1 - MARGIN_X, fy, &right, 15, DARK, Anchor::RM);
        let t = self.book.chapter_title(self.chapter);
        let t = cx.ellipsize(&t, Face::Sans, 15, a.w() - 2 * MARGIN_X - rw - 20);
        cx.text(a.x0 + MARGIN_X, fy, &t, 15, DARK, Anchor::LM);
        // tap zones: left third = back, top-middle = menu, everything else = forward
        let third = a.w() / 3;
        cx.hit(Rect::new(a.x0, a.y0, a.x1, a.y1), Hit::App(ID_NEXT), Some(Hit::App(ID_WORD)), "next page");
        cx.hit(Rect::new(a.x0, a.y0, a.x0 + third, a.y1), Hit::App(ID_PREV), Some(Hit::App(ID_WORD)), "previous page");
        cx.hit(Rect::new(a.x0 + third, a.y0, a.x1 - third, a.y0 + a.h() / 6), Hit::App(ID_MENU), None, "menu");
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        match id {
            ID_NEXT => self.next(sys),
            ID_PREV => self.prev(sys),
            ID_MENU => {
                let mut items: Vec<String> = vec!["Chapters".into(), "Font size".into()];
                items.push(if self.save_html.is_some() { "Save for later".into() } else { "Open in KOReader".into() });
                items.push("Back to library".into());
                items.push("Close menu".into());
                let heading = if self.book.author.is_empty() { self.book.title.clone() }
                              else { format!("{} · {}", self.book.title, self.book.author) };
                Cmd::Dialog(Dialog::menu(TAG_MENU, &heading, items))
            }
            ID_WORD => {
                let (x, y) = sys.last_touch;
                let hit = self.words.iter().find(|(r, _, _)| r.contains(x, y)).cloned();
                match hit {
                    Some((_, off, w)) => {
                        let word: String = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'').to_string();
                        self.sel_word = Some((off, word.clone()));
                        Cmd::Dialog(Dialog::menu(TAG_WORD, &format!("“{word}”"),
                            vec![format!("Define “{word}”"), "Highlight sentence".into(), "Cancel".into()]))
                    }
                    None => self.action(ID_MENU, false, sys),
                }
            }
            _ => Cmd::None,
        }
    }

    fn gesture(&mut self, g: &Gesture, sys: &mut Sys) -> Cmd {
        match g.kind {
            Kind::SwipeLeft | Kind::SwipeUp => self.next(sys),
            Kind::SwipeRight | Kind::SwipeDown => {
                let c = self.prev(sys);
                if let Cmd::None = c { Cmd::Fast } else { c } // never let the edge-swipe "back" fire
            }
            _ => Cmd::None,
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, _t: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match (tag, choice) {
            (TAG_MENU, 0) => {
                let items: Vec<(String, usize)> = self.book.toc.iter().map(|c| (c.title.clone(), c.spine)).collect();
                Cmd::Push(Box::new(Chapters::new(self.book.title.clone(), items, self.chapter, self.pick.clone())))
            }
            (TAG_MENU, 1) => Cmd::Dialog(Dialog::menu(TAG_FONT, "Font size",
                FONT_SIZES.iter().map(|s| format!("{s} px{}", if *s == self.fs { "  ✓" } else { "" })).collect())),
            (TAG_MENU, 2) if self.save_html.is_some() => {
                let dir = format!("{}/documents/Saved pages", sys.us);
                let _ = std::fs::create_dir_all(&dir);
                let name: String = self.book.title.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c }).take(80).collect();
                match std::fs::write(format!("{dir}/{}.html", name.trim()), self.save_html.as_deref().unwrap_or("")) {
                    Ok(()) => Cmd::Toast("Saved to Books → Saved pages".into()),
                    Err(e) => Cmd::Toast(e.to_string()),
                }
            }
            (TAG_MENU, 2) => {
                self.save(sys);
                open_koreader(sys, Some(&self.path.clone()))
            }
            (TAG_WORD, 0) => {
                let Some((_, word)) = self.sel_word.clone() else { return Cmd::None };
                let slot = Arc::new(Mutex::new(None));
                let s2 = slot.clone();
                std::thread::spawn(move || {
                    let r = crate::highlights::define(&word);
                    *s2.lock().unwrap() = Some(r);
                });
                self.define_job = Some(slot);
                Cmd::Toast("Looking up…".into())
            }
            (TAG_WORD, 1) => {
                let Some((off, _)) = self.sel_word.clone() else { return Cmd::None };
                let sentence = self.sentence_at(off);
                if sentence.is_empty() {
                    return Cmd::None;
                }
                let place = self.book.chapter_title(self.chapter);
                crate::highlights::add(sys, &self.book.title, &place, &sentence);
                Cmd::Toast("Highlighted".into())
            }
            (TAG_WORD, _) => Cmd::None,
            (TAG_MENU, 3) => Cmd::Pop,
            (TAG_FONT, i) => {
                if let Some(&s) = FONT_SIZES.get(i) {
                    self.want_offset = Some(self.start_offset());
                    self.fs = s;
                    self.laid_out = None;
                    sys.config.set("reader_font", &s.to_string());
                }
                Cmd::Full
            }
            _ => Cmd::Full,
        }
    }

    fn busy(&self) -> bool {
        self.define_job.is_some()
    }

    fn poll(&mut self, _sys: &mut Sys) -> Cmd {
        let done = self.define_job.as_ref().and_then(|j| j.lock().unwrap().take());
        let Some(done) = done else { return Cmd::None };
        self.define_job = None;
        let word = self.sel_word.as_ref().map(|w| w.1.clone()).unwrap_or_default();
        match done {
            Ok(text) => Cmd::Dialog(Dialog::message(&word, &text)),
            Err(e) => Cmd::Dialog(Dialog::message(&word, &e)),
        }
    }

    fn on_show(&mut self, sys: &mut Sys) {
        let picked = self.pick.lock().unwrap().take();
        if let Some(c) = picked {
            self.goto_chapter(c, false);
            save_position(sys, &self.path, c, 0);
        }
    }

    fn on_hide(&mut self, sys: &mut Sys) {
        if self.laid_out.is_some() {
            self.save(sys);
        }
    }
}

// --- Chapter list -------------------------------------------------------------------------------
pub struct Chapters {
    title: String,
    items: Vec<(String, usize)>,
    current: usize,
    pager: Pager,
    pick: Arc<Mutex<Option<usize>>>,
}

impl Chapters {
    fn new(title: String, items: Vec<(String, usize)>, current: usize, pick: Arc<Mutex<Option<usize>>>) -> Chapters {
        let mut pager = Pager::new(60);
        // open on the page containing the current chapter (approx. 10 rows per page)
        let idx = items.iter().rposition(|(_, s)| *s <= current).unwrap_or(0);
        pager.page = idx / 10;
        Chapters { title, items, current, pager, pick }
    }
}

impl App for Chapters {
    fn title(&self) -> String {
        format!("Chapters · {}", self.title)
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let items = &self.items;
        let current = self.current;
        let cur_idx = items.iter().rposition(|(_, s)| *s <= current);
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 8, a.x1, a.y1), items.len(), "No chapters", |cx, r, i| {
            let here = Some(i) == cur_idx;
            let face = if here { Face::Bold } else { Face::Sans };
            let t = cx.ellipsize(&items[i].0, face, 21, r.w() - 70);
            cx.c.text(cx.f, r.x0 + 24, r.cy(), &t, face, 21, BLACK, Anchor::LM);
            if here {
                cx.text(r.x1 - 24, r.cy(), "◀", 18, BLACK, Anchor::RM);
            }
            cx.hit(r, Hit::App(i as u32), None, &items[i].0);
        });
    }

    fn action(&mut self, id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        if let Some((_, spine)) = self.items.get(id as usize) {
            *self.pick.lock().unwrap() = Some(*spine);
            return Cmd::Pop;
        }
        Cmd::None
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}
