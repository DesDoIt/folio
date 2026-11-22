//! Simple built-in book reader (EPUB, TXT): full-screen pages, tap/swipe to turn,
//! top-middle tap for a menu. Positions are saved per book as (chapter, character offset),
//! so they survive font-size changes.

use crate::canvas::*;
use crate::epub::{Block, BlockKind, Book};
use crate::input::{Gesture, Kind};
use crate::sys::Sys;
use crate::ui::*;
use std::collections::HashMap;
use std::sync::Arc as Rc;
use std::sync::{Arc, Mutex};

const MARGIN_X: i32 = 36; // footer / chrome margin
const MARGIN_TOP: i32 = 34;

/// Reader layout preferences (margins, line spacing, hyphenation).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Layout {
    pub margin: i32,
    pub spacing: i32, // line height in % of the font size
    pub hyphenate: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { margin: 36, spacing: 145, hyphenate: true }
    }
}

pub const MARGINS: [(&str, i32); 3] = [("Narrow", 20), ("Normal", 36), ("Wide", 60)];
pub const SPACINGS: [(&str, i32); 3] = [("Compact", 130), ("Normal", 145), ("Relaxed", 165)];

impl Layout {
    pub fn from_config(sys: &Sys) -> Layout {
        let d = Layout::default();
        Layout {
            margin: sys.config.get("reader_margin").parse().unwrap_or(d.margin),
            spacing: sys.config.get("reader_spacing").parse().unwrap_or(d.spacing),
            hyphenate: sys.config.get("reader_hyphenate") != "no",
        }
    }
}
const FOOTER_H: i32 = 40;
pub const FONT_SIZES: [u16; 6] = [16, 18, 20, 22, 24, 28];

struct Word {
    text: String,
    face: Face,
    w: i32,
    offset: usize,
    link: Option<u32>,
    /// no space before it in the text ("paper" + "." when the link ends before the full stop):
    /// drawn without a gap and kept on the same line as the word before
    glue: bool,
}

pub struct Line {
    words: Vec<Word>,
    img: Option<Rc<crate::image::Gray>>,
    lh: i32,
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
        BlockKind::Rule | BlockKind::Image => Style { indent: 0, before: f / 2, after: f / 2, justify: false, ..base },
    }
}

/// Lay out a chapter's blocks into pages of the given area.
/// Decoded, page-fitted images of a chapter, keyed by zip path.
pub type Images = HashMap<String, Rc<crate::image::Gray>>;

pub fn paginate(blocks: &[Block], fonts: &mut Fonts, fs: u16, area: Rect, lay: Layout, images: &Images) -> Vec<Page> {
    let mut pages = vec![Page { lines: vec![], start: 0 }];
    let top = area.y0 + MARGIN_TOP;
    let bottom = area.y1 - FOOTER_H;
    let margin = lay.margin;
    let width = area.w() - 2 * margin;
    let mut y = top;
    for (bi, b) in blocks.iter().enumerate() {
        let st = style(b.kind, fs);
        let line_h = st.size as i32 * lay.spacing / 100;
        if y > top {
            y += st.before;
        }
        if b.kind == BlockKind::Image {
            let Some(img) = images.get(&b.runs[0].text) else { continue };
            let h = img.h + 12;
            if y > top && y + h > bottom {
                pages.push(Page { lines: vec![], start: b.start });
                y = top;
            }
            let x0 = area.x0 + (area.w() - img.w) / 2;
            pages.last_mut().unwrap().lines.push(Line { words: vec![], img: Some(img.clone()), lh: h, y, x0, avail: img.w,
                size: st.size, justify: false, center: false, rule: false });
            y += h + 8;
            continue;
        }
        if b.kind == BlockKind::Rule {
            if y + line_h > bottom {
                pages.push(Page { lines: vec![], start: b.start });
                y = top;
            }
            pages.last_mut().unwrap().lines.push(Line { words: vec![], img: None, lh: line_h, y, x0: area.x0 + margin, avail: width,
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
            words.push(Word { text: st.prefix.trim_end().into(), face: st.face, w: fonts.width(st.prefix.trim_end(), st.face, st.size), offset: off, link: None, glue: false });
        }
        let mut after_space = true;
        for run in &b.runs {
            let face = if st.face == Face::Mono { Face::Mono } else if run.bold { Face::Bold }
                       else if run.italic { Face::Italic } else { st.face };
            for (i, w) in run.text.split(' ').enumerate() {
                if i > 0 {
                    off += 1;
                }
                if w.is_empty() {
                    continue;
                }
                let ww = fonts.width(w, face, st.size);
                let glue = i == 0 && !after_space && !words.is_empty();
                words.push(Word { text: w.to_string(), face, w: ww, offset: off, link: run.link, glue });
                off += w.chars().count();
            }
            if !run.text.is_empty() {
                after_space = run.text.ends_with(' ');
            }
        }
        let space = fonts.width(" ", st.face, st.size).max(3);
        // list items: wrapped lines hang under the text, not the bullet
        let hang = if st.prefix.is_empty() { 0 } else { fonts.width(st.prefix, st.face, st.size) };
        let mut i = 0;
        let mut first = true;
        while i < words.len() {
            let indent = if first && st.indent > 0 && bi > 0 { st.indent } else if !first { hang } else { 0 };
            let x0 = area.x0 + margin + st.left + indent;
            let avail = width - st.left - indent;
            let mut line_words = vec![];
            let mut used = 0;
            while i < words.len() {
                let w = &words[i];
                // the word plus any fragments glued to it move to the next line together
                let mut k = i + 1;
                let mut cluster = w.w;
                while k < words.len() && words[k].glue {
                    cluster += words[k].w;
                    k += 1;
                }
                let gap = if line_words.is_empty() || w.glue { 0 } else { space };
                if used + gap + cluster <= avail || line_words.is_empty() && w.w <= avail {
                    let end = if used + gap + cluster <= avail { k } else { i + 1 };
                    used += gap;
                    for j in i..end {
                        used += words[j].w;
                        line_words.push(std::mem::replace(&mut words[j], Word { text: String::new(), face: st.face, w: 0, offset: 0, link: None, glue: false }));
                    }
                    i = end;
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
                    line_words.push(Word { text: head, face, w: hw, offset, link, glue: false });
                    words[i] = Word { text: tail, face, w: tw, offset: offset + cut, link, glue: false };
                    break;
                } else {
                    // hyphenate the word that doesn't fit, if part of it does
                    if lay.hyphenate && st.justify {
                        let room = avail - used - if w.glue { 0 } else { space };
                        let core: &str = w.text.trim_end_matches(|c: char| !c.is_alphanumeric());
                        let chars: Vec<char> = core.chars().collect();
                        for &p in crate::hyph::breaks(core).iter().rev() {
                            let head: String = chars[..p].iter().collect::<String>() + "-";
                            let hw = fonts.width(&head, w.face, st.size);
                            if hw <= room {
                                let tail: String = w.text.chars().skip(p).collect();
                                let (face, offset, link) = (w.face, w.offset, w.link);
                                let tw = fonts.width(&tail, face, st.size);
                                let glue = w.glue;
                                line_words.push(Word { text: head, face, w: hw, offset, link, glue });
                                words[i] = Word { text: tail, face, w: tw, offset: offset + p, link, glue: false };
                                break;
                            }
                        }
                    }
                    break;
                }
            }
            if y + line_h > bottom {
                let start = line_words.first().map_or(b.start, |w| w.offset);
                pages.push(Page { lines: vec![], start });
                y = top;
            }
            let last_line = i >= words.len();
            pages.last_mut().unwrap().lines.push(Line { words: line_words, img: None, lh: line_h, y, x0, avail, size: st.size,
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
        if let Some(img) = &line.img {
            cx.c.blit(&img.px, img.w, img.h, line.x0, line.y);
            continue;
        }
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
        let gaps = line.words.iter().skip(1).filter(|w| !w.glue).count() as i32;
        let total: i32 = line.words.iter().map(|w| w.w).sum::<i32>() + space * gaps;
        let mut gap = space as f32;
        let mut x = line.x0 as f32;
        if line.center {
            x = (line.x0 + (line.avail - total) / 2) as f32;
        } else if line.justify && gaps > 0 {
            let g = (line.avail - (total - space * gaps)) as f32 / gaps as f32;
            // justify unless the gaps would be absurd (very few words on a wide line)
            if g <= space as f32 * 4.0 || line.size >= 24 && g <= space as f32 * 6.0 {
                gap = g;
            }
        }
        let lh = line.lh;
        for (j, w) in line.words.iter().enumerate() {
            if j > 0 && !w.glue {
                x += gap;
            }
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
            x += w.w as f32;
        }
    }
    PageGeom { links, words }
}

// --- saved positions ---------------------------------------------------------------------------
fn positions_path(sys: &Sys) -> String {
    format!("{}/kos/reading.txt", sys.us)
}

pub(crate) fn load_positions(sys: &Sys) -> HashMap<String, (usize, usize)> {
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

pub(crate) fn save_position(sys: &Sys, path: &str, chapter: usize, offset: usize) {
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
    layout: Layout,
    laid_out: Option<(usize, u16, Rect, Layout)>,
    want_offset: Option<usize>,
    want_last: bool,
    pick: Arc<Mutex<Option<(usize, usize)>>>,
    /// link targets of the current chapter and where they are on the page
    links: Vec<String>,
    link_boxes: Vec<(Rect, u32)>,
    /// footnote target waiting for "Go there"
    note_target: Option<(usize, usize)>,
    /// progress sync: remote position offered on open, our position pushed on close
    sync_pull: Option<Arc<Mutex<Option<(f64, String, String)>>>>,
    sync_offer: Option<(usize, usize)>,
    synced_once: bool,
    /// HTML to keep if the user picks "Save for later" (articles, web pages)
    save_html: Option<String>,
    /// where a web article came from (relative links resolve against it)
    base_url: Option<String>,
    words: Vec<(Rect, usize, String)>,
    sel_word: Option<(usize, String)>,
    define_job: Option<Arc<Mutex<Option<Result<String, String>>>>>,
    last_turn: std::time::Instant,
    menu: Vec<MenuAct>,
    /// The on-screen toolbar (back, home, chapters…), shown by tapping the top of the page.
    toolbar: bool,
}

#[derive(Clone, Copy)]
enum MenuAct {
    Chapters,
    Bookmark,
    Bookmarks,
    Search,
    FontSize,
    Layout,
    SaveForLater,
    Library,
}

const ID_WORD: u32 = 4;
const TAG_WORD: u32 = 3;
const TAG_LAYOUT: u32 = 4;
const TAG_NOTE: u32 = 5;
const TAG_SEARCH: u32 = 6;
const TAG_SYNC: u32 = 7;
const ID_LINK: u32 = 0x4000_0000;
const ID_PREV: u32 = 1;
const ID_NEXT: u32 = 2;
const ID_MENU: u32 = 3;
const ID_BAR: u32 = 5; // show/hide the toolbar
const ID_BAR_CHAPTERS: u32 = 6;
const ID_BAR_BOOKMARK: u32 = 7;
const ID_BAR_SEARCH: u32 = 8;
const ID_BAR_FONT: u32 = 9;
const BAR_TOP_H: i32 = 64;
const BAR_BOTTOM_H: i32 = 118;
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
        Reader { path: key.into(), book, chapter, pages: vec![], page: 0, fs, layout: Layout::from_config(sys), laid_out: None,
                 want_offset: Some(offset), want_last: false, pick: Arc::new(Mutex::new(None)), sync_pull: None, sync_offer: None, synced_once: false, links: vec![],
                 link_boxes: vec![], note_target: None, save_html: None, base_url: None,
                 words: vec![], sel_word: None, define_job: None, last_turn: std::time::Instant::now(),
                 menu: vec![], toolbar: false }
    }

    /// Run one menu action directly (from the toolbar buttons).
    fn do_act(&mut self, act: MenuAct, sys: &mut Sys) -> Cmd {
        self.toolbar = false;
        self.menu = vec![act];
        self.dialog(TAG_MENU, 0, String::new(), sys)
    }

    /// Go to a chapter and character offset.
    fn jump(&mut self, chapter: usize, offset: usize) {
        self.chapter = chapter.min(self.book.spine.len().saturating_sub(1));
        self.laid_out = None;
        self.want_offset = Some(offset);
        self.want_last = false;
        self.page = 0;
    }

    fn bookmarks_path(sys: &Sys) -> String {
        format!("{}/kos/book-bookmarks.txt", sys.us)
    }

    /// This book's bookmarks: (chapter, offset, label).
    fn bookmarks(&self, sys: &Sys) -> Vec<(usize, usize, String)> {
        std::fs::read_to_string(Self::bookmarks_path(sys)).unwrap_or_default().lines().filter_map(|l| {
            let p: Vec<&str> = l.splitn(4, '\t').collect();
            (p.len() == 4 && p[0] == self.path).then(|| Some((p[1].parse().ok()?, p[2].parse().ok()?, p[3].to_string())))
                .flatten()
        }).collect()
    }

    fn page_range(&self) -> (usize, usize) {
        let start = self.start_offset();
        let end = self.pages.get(self.page + 1).map_or(usize::MAX, |p| p.start);
        (start, end)
    }

    fn is_bookmarked(&self, sys: &Sys) -> bool {
        let (s, e) = self.page_range();
        self.bookmarks(sys).iter().any(|(c, o, _)| *c == self.chapter && *o >= s && *o < e)
    }

    /// Add or remove a bookmark on the current page; returns true if one was added.
    fn toggle_bookmark(&self, sys: &Sys) -> bool {
        let path = Self::bookmarks_path(sys);
        let all = std::fs::read_to_string(&path).unwrap_or_default();
        let (s, e) = self.page_range();
        let mut removed = false;
        let mut kept: Vec<String> = vec![];
        for l in all.lines() {
            let p: Vec<&str> = l.splitn(4, '\t').collect();
            let here = p.len() == 4 && p[0] == self.path && p[1].parse::<usize>().ok() == Some(self.chapter)
                && p[2].parse::<usize>().map_or(false, |o| o >= s && o < e);
            if here {
                removed = true;
            } else {
                kept.push(l.to_string());
            }
        }
        if !removed {
            let label: String = self.words.iter().take(10).map(|w| w.2.as_str()).collect::<Vec<_>>().join(" ");
            kept.push(format!("{}\t{}\t{}\t{}", self.path, self.chapter, s, label.replace('\t', " ")));
        }
        let _ = std::fs::create_dir_all(format!("{}/kos", sys.us));
        let _ = std::fs::write(&path, kept.join("\n") + "\n");
        !removed
    }

    /// Case-insensitive search through every chapter: (snippet, chapter title, chapter, offset).
    fn search(&mut self, q: &str) -> Vec<(String, String, usize, usize)> {
        let mut out = vec![];
        let qn = q.chars().count();
        'outer: for ch in 0..self.book.spine.len() {
            let blocks = self.book.chapter_blocks(ch);
            let title = self.book.chapter_title(ch);
            for b in blocks.iter().filter(|b| b.kind != BlockKind::Image) {
                let text: String = b.runs.iter().map(|r| r.text.as_str()).collect();
                let chars: Vec<char> = text.chars().collect();
                let lower: Vec<char> = text.to_lowercase().chars().collect();
                if lower.len() != chars.len() {
                    continue; // rare case-mapping length change: skip rather than misplace
                }
                let qc: Vec<char> = q.chars().collect();
                let mut i = 0;
                while i + qn <= lower.len() {
                    if lower[i..i + qn] == qc[..] {
                        let s = i.saturating_sub(30);
                        let e = (i + qn + 40).min(chars.len());
                        let snippet = format!("{}{}{}", if s > 0 { "…" } else { "" },
                                              chars[s..e].iter().collect::<String>(), if e < chars.len() { "…" } else { "" });
                        out.push((snippet, title.clone(), ch, b.start + i));
                        if out.len() >= 200 {
                            break 'outer;
                        }
                        i += qn;
                    } else {
                        i += 1;
                    }
                }
            }
        }
        out
    }

    /// The web address an article came from: its links then open in Wikipedia or the Web app.
    pub fn set_base_url(&mut self, url: &str) {
        self.base_url = Some(url.to_string());
    }

    /// A link out of the book: Wikipedia articles open in Wikipedia, anything else in the Web app.
    fn follow_web_link(&self, href: &str, sys: &Sys) -> Cmd {
        let url = match &self.base_url {
            Some(base) => crate::web::url_join(base, href),
            None if href.contains("://") => href.to_string(),
            None => return Cmd::Toast("This link points outside the book".into()),
        };
        if url.starts_with("mailto:") {
            return Cmd::Toast(format!("Email: {}", url.trim_start_matches("mailto:")));
        }
        if !url.starts_with("http") {
            return Cmd::Toast("This link can't be opened".into());
        }
        if let Some((lang, title)) = crate::wiki::article_link(&url) {
            return crate::wiki::open_article(&lang, &title, sys);
        }
        if url.contains("wikipedia.org/") && url.contains("redlink=1") {
            return Cmd::Toast("Wikipedia has no article on this yet".into());
        }
        Cmd::Push(Box::new(crate::web::Web::open(url, sys)))
    }

    /// Tap on a link: short targets (footnotes) pop up; others (table of contents) jump; web links
    /// leave the book.
    fn follow_link(&mut self, idx: usize, sys: &Sys) -> Cmd {
        let Some(href) = self.links.get(idx).cloned() else { return Cmd::None };
        let inside = if self.base_url.is_some() && !href.starts_with('#') { None } else { self.book.resolve_link(self.chapter, &href) };
        let Some((ch, frag)) = inside else {
            return self.follow_web_link(&href, sys);
        };
        let doc = self.book.chapter_doc(ch);
        let off = match frag.as_ref().map(|f| doc.anchors.get(f).copied()) {
            Some(Some(o)) => o,
            // a note or section this copy doesn't have: don't jump to the chapter start
            Some(None) if ch == self.chapter => return Cmd::Toast("That part isn't in this copy".into()),
            _ => 0,
        };
        let block = doc.blocks.iter().filter(|b| b.kind != BlockKind::Image).find(|b| b.start + b.chars() > off);
        let text: String = block.map(|b| b.runs.iter().map(|r| r.text.as_str()).collect()).unwrap_or_default();
        if frag.is_some() && !text.is_empty() && text.chars().count() < 700 {
            self.note_target = Some((ch, off));
            return Cmd::Dialog(Dialog::confirm(TAG_NOTE, "Note", &text, "Go there"));
        }
        self.jump(ch, off);
        Cmd::Full
    }

    fn layout_menu(&self) -> Cmd {
        let m = MARGINS.iter().find(|x| x.1 == self.layout.margin).map_or("Normal", |x| x.0);
        let s = SPACINGS.iter().find(|x| x.1 == self.layout.spacing).map_or("Normal", |x| x.0);
        Cmd::Dialog(Dialog::menu(TAG_LAYOUT, "Layout", vec![
            format!("Margins: {m}"), format!("Line spacing: {s}"),
            format!("Hyphenation: {}", if self.layout.hyphenate { "on" } else { "off" }), "Done".into()]))
    }

    /// Book-level position (0..1) for syncing: chapter + fraction of the chapter.
    fn fraction(&self) -> f64 {
        let n = self.book.spine.len().max(1) as f64;
        let within = if self.pages.len() > 1 { self.page as f64 / self.pages.len() as f64 } else { 0.0 };
        ((self.chapter as f64 + within) / n).min(1.0)
    }

    /// Ask the sync server for this book's last position (background thread).
    fn start_sync_pull(&mut self, sys: &Sys) {
        if !std::path::Path::new(&self.path).is_file() {
            return;
        }
        let Some(acct) = crate::sync::Account::from_config(&sys.config) else { return };
        let path = self.path.clone();
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            if let Some(doc) = crate::sync::document_id(&path) {
                if let Ok(Some(p)) = acct.pull(&doc) {
                    *s2.lock().unwrap() = Some(p);
                }
            }
        });
        self.sync_pull = Some(slot);
    }

    fn sync_push(&self, sys: &Sys) {
        if !std::path::Path::new(&self.path).is_file() {
            return;
        }
        let Some(acct) = crate::sync::Account::from_config(&sys.config) else { return };
        let (path, frac, chapter) = (self.path.clone(), self.fraction(), self.chapter);
        let device = sys.config.get("sync_device_id").to_string();
        std::thread::spawn(move || {
            if let Some(doc) = crate::sync::document_id(&path) {
                if let Err(e) = acct.push(&doc, frac, chapter, &device) {
                    crate::sys::log(&format!("sync push: {e}"));
                }
            }
        });
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
        self.toolbar = false;
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
        self.toolbar = false;
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
        // remember the book for the "current book" sleep screen (files only, not articles)
        if std::path::Path::new(&self.path).is_file() {
            let line = format!("{}\t{}\t{}\t{}\n", self.path, self.book.title.replace('\t', " "),
                               self.book.author.replace('\t', " "), self.progress());
            let _ = std::fs::write(format!("{}/kos/last-book.txt", sys.us), line);
            crate::covers::save_progress(sys, &self.path, self.progress(), &self.book.title, &self.book.author);
        }
    }

    fn progress(&self) -> u32 {
        let n = self.book.spine.len().max(1) as f32;
        let within = if self.pages.len() > 1 { self.page as f32 / self.pages.len() as f32 } else { 0.0 };
        (((self.chapter as f32 + within) / n) * 100.0) as u32
    }
}

impl Reader {
    fn render_toolbar(&mut self, cx: &mut Ctx, a: Rect, sys: &Sys) {
        // anywhere outside the bars closes the toolbar
        cx.hit(a, Hit::App(ID_BAR), Some(Hit::App(ID_BAR)), "close toolbar");
        let b = Btn::size(20);
        // top: back, home, title, text size, full menu
        let top = Rect::new(a.x0, a.y0, a.x1, a.y0 + BAR_TOP_H);
        cx.c.fill(top, WHITE);
        cx.c.hline(a.x0, a.x1, top.y1 - 2, 2, BLACK);
        cx.button(Rect::new(6, top.y0 + 6, 70, top.y1 - 8), "‹", Some(Hit::Back), b);
        cx.button(Rect::new(76, top.y0 + 6, 140, top.y1 - 8), "⌂", Some(Hit::Home), Btn { size: 24, ..b });
        cx.button(Rect::new(a.x1 - 134, top.y0 + 6, a.x1 - 72, top.y1 - 8), "Aa", Some(Hit::App(ID_BAR_FONT)), b);
        cx.button(Rect::new(a.x1 - 66, top.y0 + 6, a.x1 - 6, top.y1 - 8), "☰", Some(Hit::App(ID_MENU)), Btn { size: 24, ..b });
        let t = cx.ellipsize(&self.book.title, Face::Bold, 20, a.w() - 152 - 146);
        cx.bold(152, top.cy() - 2, &t, 20, BLACK, Anchor::LM);
        // bottom: where you are, and the common actions
        let bot = Rect::new(a.x0, a.y1 - BAR_BOTTOM_H, a.x1, a.y1);
        cx.c.fill(bot, WHITE);
        cx.c.hline(a.x0, a.x1, bot.y0, 2, BLACK);
        let right = format!("{} / {}  ·  {}%", self.page + 1, self.pages.len().max(1), self.progress());
        let rw = cx.text(a.x1 - 16, bot.y0 + 20, &right, 15, DARK, Anchor::RM);
        let ch = self.book.chapter_title(self.chapter);
        let ch = cx.ellipsize(&ch, Face::Bold, 16, a.w() - 48 - rw);
        cx.bold(a.x0 + 16, bot.y0 + 20, &ch, 16, BLACK, Anchor::LM);
        let bar = Rect::new(a.x0 + 16, bot.y0 + 36, a.x1 - 16, bot.y0 + 44);
        cx.c.round_rect(bar, 4, WHITE, 1, BLACK);
        let w = (bar.w() - 2) * (self.progress() as i32).min(100) / 100;
        if w > 0 {
            cx.c.fill(Rect::new(bar.x0 + 1, bar.y0 + 1, bar.x0 + 1 + w, bar.y1 - 1), BLACK);
        }
        let marked = self.is_bookmarked(sys);
        let labels = [("Chapters", ID_BAR_CHAPTERS), (if marked { "★ Marked" } else { "☆ Bookmark" }, ID_BAR_BOOKMARK),
                      ("Search", ID_BAR_SEARCH)];
        let bw = (a.w() - 16 - 8 * (labels.len() as i32 - 1)) / labels.len() as i32;
        for (k, (l, id)) in labels.iter().enumerate() {
            let x = a.x0 + 8 + k as i32 * (bw + 8);
            cx.button(Rect::new(x, bot.y0 + 56, x + bw, bot.y1 - 10), l, Some(Hit::App(*id)), b);
        }
    }
}

impl App for Reader {
    fn title(&self) -> String {
        self.book.title.clone()
    }

    fn fullscreen(&self) -> bool {
        true
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        if self.laid_out != Some((self.chapter, self.fs, a, self.layout)) {
            let doc = self.book.chapter_doc(self.chapter);
            self.links = doc.links;
            let blocks = doc.blocks;
            let (max_w, max_h) = (a.w() - 2 * self.layout.margin, a.h() - MARGIN_TOP - FOOTER_H - 10);
            let mut images: Images = HashMap::new();
            for b in blocks.iter().filter(|b| b.kind == BlockKind::Image) {
                let p = &b.runs[0].text;
                if images.contains_key(p) {
                    continue;
                }
                if let Some(g) = self.book.file(p).and_then(|bytes| crate::image::decode(&bytes).ok()) {
                    if g.w >= 24 && g.h >= 24 {
                        // never upscale small images more than 2x; always fit the page
                        let s = (max_w as f32 / g.w as f32).min(max_h as f32 / g.h as f32).min(2.0);
                        let fitted = crate::image::resize(&g, (g.w as f32 * s) as i32, (g.h as f32 * s) as i32);
                        images.insert(p.clone(), Rc::new(fitted));
                    }
                }
            }
            self.pages = paginate(&blocks, cx.f, self.fs, a, self.layout, &images);
            self.page = if self.want_last {
                self.pages.len() - 1
            } else if let Some(off) = self.want_offset.take() {
                self.pages.iter().rposition(|p| p.start <= off).unwrap_or(0)
            } else {
                self.page.min(self.pages.len() - 1)
            };
            self.want_last = false;
            self.laid_out = Some((self.chapter, self.fs, a, self.layout));
        }
        if let Some(p) = self.pages.get(self.page) {
            let geom = draw_page(cx, p);
            self.words = geom.words;
            self.link_boxes = geom.links;
            // bookmark ribbon
            if self.is_bookmarked(sys) {
                let x = a.x1 - 44;
                cx.c.fill(Rect::new(x, a.y0, x + 22, a.y0 + 34), BLACK);
                for k in 0..10 {
                    cx.c.fill(Rect::new(x + 11 - k, a.y0 + 34 - 10 + k, x + 11 + k + 1, a.y0 + 34 - 10 + k + 1), WHITE);
                }
            }
        }
        // footer: chapter title, page within chapter, book progress
        let fy = a.y1 - FOOTER_H / 2 - 2;
        cx.c.hline(a.x0 + MARGIN_X, a.x1 - MARGIN_X, a.y1 - FOOTER_H + 4, 1, LIGHT);
        let right = format!("{} / {}  ·  {}%", self.page + 1, self.pages.len(), self.progress());
        let rw = cx.text(a.x1 - MARGIN_X, fy, &right, 15, DARK, Anchor::RM);
        let t = self.book.chapter_title(self.chapter);
        let t = cx.ellipsize(&t, Face::Sans, 15, a.w() - 2 * MARGIN_X - rw - 20);
        cx.text(a.x0 + MARGIN_X, fy, &t, 15, DARK, Anchor::LM);
        if self.toolbar {
            self.render_toolbar(cx, a, sys);
            return;
        }
        // tap zones: top strip = toolbar, left third = back, everything else = forward
        let third = a.w() / 3;
        cx.hit(Rect::new(a.x0, a.y0, a.x1, a.y1), Hit::App(ID_NEXT), Some(Hit::App(ID_WORD)), "next page");
        cx.hit(Rect::new(a.x0, a.y0, a.x0 + third, a.y1), Hit::App(ID_PREV), Some(Hit::App(ID_WORD)), "previous page");
        cx.hit(Rect::new(a.x0, a.y0, a.x1, a.y0 + a.h() / 7), Hit::App(ID_BAR), Some(Hit::App(ID_BAR)), "toolbar");
        for (r, l) in &self.link_boxes {
            // small note markers are hard to hit: give them at least 44 px
            let pad_x = ((44 - r.w()) / 2).max(0);
            let pad_y = ((44 - r.h()) / 2).max(0);
            let rr = Rect::new(r.x0 - pad_x, r.y0 - pad_y, r.x1 + pad_x, r.y1 + pad_y);
            cx.hit(rr, Hit::App(ID_LINK | *l), Some(Hit::App(ID_WORD)), "book-link");
        }
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        match id {
            l if l & ID_LINK != 0 => self.follow_link((l & !ID_LINK) as usize, sys),
            ID_NEXT => self.next(sys),
            ID_PREV => self.prev(sys),
            ID_BAR => {
                self.toolbar = !self.toolbar;
                if self.toolbar { Cmd::Fast } else { Cmd::Redraw }
            }
            ID_BAR_CHAPTERS => self.do_act(MenuAct::Chapters, sys),
            ID_BAR_BOOKMARK => {
                let c = self.do_act(MenuAct::Bookmark, sys);
                self.toolbar = true; // keep the bar so the star change is visible
                c
            }
            ID_BAR_SEARCH => self.do_act(MenuAct::Search, sys),
            ID_BAR_FONT => self.do_act(MenuAct::FontSize, sys),
            ID_MENU => {
                self.toolbar = false;
                self.menu = vec![MenuAct::Chapters, MenuAct::Bookmark, MenuAct::Bookmarks, MenuAct::Search,
                                 MenuAct::FontSize, MenuAct::Layout];
                // in-memory articles (news, web pages) can be saved as books
                if self.save_html.is_some() {
                    self.menu.push(MenuAct::SaveForLater);
                }
                self.menu.push(MenuAct::Library);
                let marked = self.is_bookmarked(sys);
                let nbm = self.bookmarks(sys).len();
                let items: Vec<String> = self.menu.iter().map(|m| match m {
                    MenuAct::Bookmark => if marked { "Remove bookmark".to_string() } else { "Bookmark this page".to_string() },
                    MenuAct::Bookmarks => format!("Bookmarks ({nbm})"),
                    MenuAct::Search => "Search in book".to_string(),
                    other => match other {
                    MenuAct::Chapters => "Chapters",
                    MenuAct::FontSize => "Font size",
                    MenuAct::Layout => "Layout: margins, spacing",
                    MenuAct::SaveForLater => "Save for later",
                    MenuAct::Library => "Back to library",
                    _ => "",
                    }.to_string(),
                }).collect();
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
                    None => {
                        self.toolbar = true;
                        Cmd::Fast
                    }
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
        let act = if tag == TAG_MENU { self.menu.get(choice).copied() } else { None };
        match (tag, choice) {
            (TAG_MENU, _) => match act {
                Some(MenuAct::Chapters) => {
                    let items: Vec<(String, String, usize, usize)> =
                        self.book.toc.iter().map(|c| (c.title.clone(), String::new(), c.spine, 0)).collect();
                    let mark = items.iter().rposition(|i| i.2 <= self.chapter);
                    Cmd::Push(Box::new(JumpList::new(format!("Chapters · {}", self.book.title), items, mark,
                                                     self.pick.clone(), "No chapters")))
                }
                Some(MenuAct::Bookmark) => {
                    let on = self.toggle_bookmark(sys);
                    Cmd::Batch(vec![Cmd::Full, Cmd::Toast(if on { "Bookmarked".into() } else { "Bookmark removed".into() })])
                }
                Some(MenuAct::Bookmarks) => {
                    let items: Vec<(String, String, usize, usize)> = self.bookmarks(sys).into_iter()
                        .map(|(c, o, label)| (label, self.book.chapter_title(c), c, o)).collect();
                    Cmd::Push(Box::new(JumpList::new("Bookmarks".into(), items, None, self.pick.clone(),
                                                     "No bookmarks yet")))
                }
                Some(MenuAct::Search) => Cmd::Dialog(Dialog::input(TAG_SEARCH, "Search in book", "", "Search")),
                Some(MenuAct::FontSize) => Cmd::Dialog(Dialog::menu(TAG_FONT, "Font size",
                    FONT_SIZES.iter().map(|s| format!("{s} px{}", if *s == self.fs { "  ✓" } else { "" })).collect())),
                Some(MenuAct::SaveForLater) => {
                    let dir = format!("{}/documents/Saved pages", sys.us);
                    let _ = std::fs::create_dir_all(&dir);
                    let name: String = self.book.title.chars().map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c }).take(80).collect();
                    match std::fs::write(format!("{dir}/{}.html", name.trim()), self.save_html.as_deref().unwrap_or("")) {
                        Ok(()) => Cmd::Toast("Saved to Books → Saved pages".into()),
                        Err(e) => Cmd::Toast(e.to_string()),
                    }
                }
                Some(MenuAct::Layout) => self.layout_menu(),
                Some(MenuAct::Library) => Cmd::Pop,
                None => Cmd::Full,
            },
            (TAG_FONT, i) => {
                if let Some(&s) = FONT_SIZES.get(i) {
                    self.want_offset = Some(self.start_offset());
                    self.fs = s;
                    self.laid_out = None;
                    sys.config.set("reader_font", &s.to_string());
                }
                Cmd::Full
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
            (TAG_SYNC, _) => match self.sync_offer.take() {
                Some((c, off)) => {
                    self.jump(c, off);
                    Cmd::Full
                }
                None => Cmd::None,
            },
            (TAG_NOTE, _) => match self.note_target.take() {
                Some((c, off)) => {
                    self.jump(c, off);
                    Cmd::Full
                }
                None => Cmd::None,
            },
            (TAG_SEARCH, _) => {
                let q = _t.trim().to_lowercase();
                if q.chars().count() < 2 {
                    return Cmd::None;
                }
                let hits = self.search(&q);
                let title = format!("“{}” · {} found", _t.trim(), hits.len());
                Cmd::Push(Box::new(JumpList::new(title, hits, None, self.pick.clone(), "Not found")))
            }
            (TAG_LAYOUT, i) if i < 3 => {
                // each tap cycles the option; keep the reading position across re-layout
                self.want_offset = Some(self.start_offset());
                match i {
                    0 => {
                        let k = MARGINS.iter().position(|x| x.1 == self.layout.margin).unwrap_or(1);
                        self.layout.margin = MARGINS[(k + 1) % MARGINS.len()].1;
                        sys.config.set("reader_margin", &self.layout.margin.to_string());
                    }
                    1 => {
                        let k = SPACINGS.iter().position(|x| x.1 == self.layout.spacing).unwrap_or(1);
                        self.layout.spacing = SPACINGS[(k + 1) % SPACINGS.len()].1;
                        sys.config.set("reader_spacing", &self.layout.spacing.to_string());
                    }
                    _ => {
                        self.layout.hyphenate = !self.layout.hyphenate;
                        sys.config.set("reader_hyphenate", if self.layout.hyphenate { "yes" } else { "no" });
                    }
                }
                self.laid_out = None;
                Cmd::Batch(vec![Cmd::Full, self.layout_menu()])
            }
            (TAG_LAYOUT, _) => Cmd::Full,
            _ => Cmd::Full,
        }
    }

    fn busy(&self) -> bool {
        self.define_job.is_some() || self.sync_pull.is_some()
    }

    fn poll(&mut self, _sys: &mut Sys) -> Cmd {
        let pulled = self.sync_pull.as_ref().and_then(|s| s.lock().unwrap().take());
        if let Some((pct, progress, device)) = pulled {
            self.sync_pull = None;
            let mine = self.fraction();
            if pct > mine + 0.01 && device != "Kindle Linux" {
                let n = self.book.spine.len().max(1);
                let ch = crate::sync::chapter_from_xpointer(&progress).unwrap_or((pct * n as f64) as usize).min(n - 1);
                let within = (pct * n as f64 - ch as f64).clamp(0.0, 1.0);
                let len: usize = self.book.chapter_blocks(ch).last().map_or(0, |b| b.start + b.chars());
                self.sync_offer = Some((ch, (within * len as f64) as usize));
                return Cmd::Dialog(Dialog::confirm(TAG_SYNC, "Continue reading?",
                    &format!("{device} is at {:.0}%. You're at {:.0}% here.", pct * 100.0, mine * 100.0), "Jump there"));
            }
        }
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
        if self.sync_pull.is_none() && self.sync_offer.is_none() && !self.synced_once {
            self.synced_once = true;
            // show the toolbar on the first few books, so people learn where it lives
            let seen: u32 = sys.config.get("reader_bar_hint").parse().unwrap_or(0);
            if seen < 3 {
                sys.config.set("reader_bar_hint", &(seen + 1).to_string());
                self.toolbar = true;
            }
            self.start_sync_pull(sys);
        }
        let picked = self.pick.lock().unwrap().take();
        if let Some((c, off)) = picked {
            self.jump(c, off);
            save_position(sys, &self.path, c, off);
        }
    }

    fn on_hide(&mut self, sys: &mut Sys) {
        if self.laid_out.is_some() {
            self.save(sys);
            self.sync_push(sys);
        }
    }
}

// --- Jump list: chapters, bookmarks, search results -----------------------------------------------
pub struct JumpList {
    title: String,
    items: Vec<(String, String, usize, usize)>, // (label, sub-label, chapter, offset)
    mark: Option<usize>,
    pager: Pager,
    pick: Arc<Mutex<Option<(usize, usize)>>>,
    empty: &'static str,
}

impl JumpList {
    pub(crate) fn new(title: String, items: Vec<(String, String, usize, usize)>, mark: Option<usize>,
           pick: Arc<Mutex<Option<(usize, usize)>>>, empty: &'static str) -> JumpList {
        let mut pager = Pager::new(64);
        pager.page = mark.unwrap_or(0) / 10; // open near the marked entry (about 10 rows per page)
        JumpList { title, items, mark, pager, pick, empty }
    }
}

impl App for JumpList {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let items = &self.items;
        let mark = self.mark;
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 8, a.x1, a.y1), items.len(), self.empty, |cx, r, i| {
            let here = Some(i) == mark;
            let face = if here { Face::Bold } else { Face::Sans };
            let (label, sub) = (&items[i].0, &items[i].1);
            let y = if sub.is_empty() { r.cy() } else { r.y0 + 20 };
            let t = cx.ellipsize(label, face, 20, r.w() - 70);
            cx.c.text(cx.f, r.x0 + 24, y, &t, face, 20, BLACK, Anchor::LM);
            if !sub.is_empty() {
                let st = cx.ellipsize(sub, Face::Sans, 14, r.w() - 70);
                cx.text(r.x0 + 24, r.y0 + 46, &st, 14, DARK, Anchor::LM);
            }
            if here {
                cx.text(r.x1 - 24, r.cy(), "◀", 18, BLACK, Anchor::RM);
            }
            cx.hit(r, Hit::App(i as u32), None, label);
        });
    }

    fn action(&mut self, id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        if let Some((_, _, ch, off)) = self.items.get(id as usize) {
            *self.pick.lock().unwrap() = Some((*ch, *off));
            return Cmd::Pop;
        }
        Cmd::None
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}
