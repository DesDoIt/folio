//! A reading session: one open book laid out for one screen. This is the whole reading API a
//! frontend needs: it opens a book, turns pages (across chapter boundaries), renders the page
//! before, at or after the current one as pixels, follows links and footnotes, finds words under
//! a finger, searches, and reports a position the frontend saves and later restores.
//!
//! The session never touches the network or decides where files live: the platform stores
//! positions and bookmarks and passes paths in.

use crate::canvas::*;
use crate::epub::{BlockKind, Book};
use crate::layout::*;
use std::collections::HashMap;
use std::sync::Arc as Rc;

/// Where a reader is in a book: a chapter (spine item) and a character offset into it.
/// Stable across font, margin and screen-size changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub chapter: usize,
    pub offset: usize,
}

/// How pages look. Sizes are in pixels of the target screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub font_size: u16,
    pub layout: Layout,
}

impl Default for Style {
    fn default() -> Self {
        Style { font_size: 20, layout: Layout::default() }
    }
}

/// Paper and ink colours (RGB) for rendering a page in colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub paper: [u8; 3],
    pub ink: [u8; 3],
    /// draw pictures as they are, not recoloured (keeps photos natural on a dark theme)
    pub natural_images: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Theme { paper: [255, 255, 255], ink: [0, 0, 0], natural_images: true }
    }
}

pub struct TocEntry {
    pub title: String,
    pub position: Position,
}

pub struct SearchHit {
    /// the match with some text around it
    pub snippet: String,
    pub chapter_title: String,
    pub position: Position,
}

/// What a tap on the page means.
#[derive(Debug, PartialEq)]
pub enum Tap {
    /// nothing tappable there
    None,
    /// an internal link: go there
    Jump(Position),
    /// a short link target (a footnote): show its text, offer to go there
    Note { text: String, target: Position },
    /// a link that leaves the book (http, mailto)
    External(String),
}

/// A word on the current page, as found under a finger.
#[derive(Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub position: Position,
    pub rect: Rect,
}

struct Laid {
    pages: Vec<Page>,
    links: Vec<String>,
}

/// A rendered page: 8-bit grayscale (0 = ink, 255 = paper) plus where pictures landed.
pub struct Rendered {
    pub canvas: Canvas,
    geom: PageGeom,
}

pub struct Session {
    book: Book,
    fonts: Fonts,
    w: i32,
    h: i32,
    style: Style,
    chapter: usize,
    page: usize,
    /// set when the current chapter must be (re)laid out; holds the offset to land on
    want: Option<usize>,
    /// a few laid-out chapters (current and neighbours), most recent last
    cache: Vec<(usize, Laid)>,
    /// hit areas of the current page, from its last render
    geom: Option<PageGeom>,
    weights: Option<Vec<f32>>,
}

const CACHE_CHAPTERS: usize = 3;

impl Session {
    /// Open a book file: EPUB, MOBI/AZW3/AZW/PRC, FB2/FB2.ZIP, RTF, DOCX, HTML or TXT.
    pub fn open(path: &str, fonts: Fonts) -> Result<Session, String> {
        Ok(Session::with_book(Book::open(path)?, fonts))
    }

    pub fn with_book(book: Book, fonts: Fonts) -> Session {
        Session { book, fonts, w: 600, h: 800, style: Style::default(), chapter: 0, page: 0, want: Some(0),
                  cache: vec![], geom: None, weights: None }
    }

    pub fn title(&self) -> &str {
        &self.book.title
    }

    pub fn author(&self) -> &str {
        &self.book.author
    }

    pub fn cover_image(&mut self) -> Option<Vec<u8>> {
        self.book.cover_image()
    }

    pub fn chapter_count(&self) -> usize {
        self.book.spine.len()
    }

    pub fn toc(&self) -> Vec<TocEntry> {
        self.book.toc.iter().map(|c| TocEntry { title: c.title.clone(), position: Position { chapter: c.spine, offset: 0 } })
            .collect()
    }

    /// Title of the table-of-contents entry covering the current page.
    pub fn chapter_title(&self) -> String {
        self.book.chapter_title(self.chapter)
    }

    /// Change the page size (pixels). Keeps the reading position.
    pub fn set_viewport(&mut self, w: i32, h: i32) {
        if (w, h) != (self.w, self.h) && w > 0 && h > 0 {
            let pos = self.position();
            self.w = w;
            self.h = h;
            self.relayout(pos);
        }
    }

    /// Change font size, margins, spacing or hyphenation. Keeps the reading position.
    pub fn set_style(&mut self, style: Style) {
        if style != self.style {
            let pos = self.position();
            self.style = style;
            self.relayout(pos);
        }
    }

    /// Change the text faces (e.g. serif to sans). Keeps the reading position.
    pub fn set_fonts(&mut self, fonts: Fonts) {
        let pos = self.position();
        self.fonts = fonts;
        self.relayout(pos);
    }

    fn relayout(&mut self, pos: Position) {
        self.cache.clear();
        self.geom = None;
        self.goto(pos);
    }

    /// The start of the current page.
    pub fn position(&mut self) -> Position {
        if let Some(offset) = self.want {
            return Position { chapter: self.chapter, offset };
        }
        let (ch, pg) = (self.chapter, self.page);
        let start = self.laid(ch).pages.get(pg).map_or(0, |p| p.start);
        Position { chapter: self.chapter, offset: start }
    }

    /// Go to the page containing this position.
    pub fn goto(&mut self, pos: Position) {
        self.chapter = pos.chapter.min(self.book.spine.len().saturating_sub(1));
        self.want = Some(pos.offset);
        self.page = 0;
        self.geom = None;
    }

    /// Lay out a chapter (or take it from the cache).
    fn laid(&mut self, chapter: usize) -> &Laid {
        if let Some(i) = self.cache.iter().position(|(c, _)| *c == chapter) {
            let entry = self.cache.remove(i);
            self.cache.push(entry);
        } else {
            let laid = self.layout_chapter(chapter);
            if self.cache.len() >= CACHE_CHAPTERS {
                self.cache.remove(0);
            }
            self.cache.push((chapter, laid));
        }
        &self.cache.last().unwrap().1
    }

    fn layout_chapter(&mut self, chapter: usize) -> Laid {
        let doc = self.book.chapter_doc(chapter);
        let lay = self.style.layout;
        let area = Rect::new(0, 0, self.w, self.h);
        let (max_w, max_h) = (area.w() - 2 * lay.margin, area.h() - lay.top - lay.bottom - 10);
        let mut images: Images = HashMap::new();
        for b in doc.blocks.iter().filter(|b| b.kind == BlockKind::Image) {
            let p = &b.runs[0].text;
            if images.contains_key(p) {
                continue;
            }
            if let Some(g) = self.book.file(p).and_then(|bytes| crate::image::decode(&bytes).ok()) {
                if g.w >= 24 && g.h >= 24 && max_w > 0 && max_h > 0 {
                    // never upscale small images more than 2x; always fit the page
                    let s = (max_w as f32 / g.w as f32).min(max_h as f32 / g.h as f32).min(2.0);
                    let fitted = crate::image::resize(&g, (g.w as f32 * s) as i32, (g.h as f32 * s) as i32);
                    images.insert(p.clone(), Rc::new(fitted));
                }
            }
        }
        let pages = paginate(&doc.blocks, &mut self.fonts, self.style.font_size, area, lay, &images);
        Laid { pages, links: doc.links }
    }

    /// Settle the current page index once the current chapter is laid out.
    fn settle(&mut self) {
        let want = self.want.take();
        let ch = self.chapter;
        let n = self.laid(ch).pages.len();
        self.page = match want {
            Some(usize::MAX) => n - 1,
            Some(off) => self.laid(ch).pages.iter().rposition(|p| p.start <= off).unwrap_or(0),
            None => self.page.min(n - 1),
        };
    }

    /// (page index within the chapter, pages in the chapter)
    pub fn page_in_chapter(&mut self) -> (usize, usize) {
        self.settle();
        let ch = self.chapter;
        (self.page, self.laid(ch).pages.len())
    }

    /// The (chapter, page) `delta` pages away from the current one, if the book has it.
    fn step(&mut self, delta: i32) -> Option<(usize, usize)> {
        self.settle();
        let (mut ch, mut pg) = (self.chapter, self.page as i64 + delta as i64);
        loop {
            let n = self.laid(ch).pages.len() as i64;
            if pg < 0 {
                if ch == 0 {
                    return None;
                }
                ch -= 1;
                pg += self.laid(ch).pages.len() as i64;
            } else if pg >= n {
                if ch + 1 >= self.book.spine.len() {
                    return None;
                }
                pg -= n;
                ch += 1;
            } else {
                return Some((ch, pg as usize));
            }
        }
    }

    /// Turn forward or back `delta` pages; false at either end of the book.
    pub fn turn(&mut self, delta: i32) -> bool {
        match self.step(delta) {
            Some((ch, pg)) => {
                self.chapter = ch;
                self.page = pg;
                self.geom = None;
                true
            }
            None => false,
        }
    }

    /// Draw the page `delta` pages from the current one (0 = current), as grayscale.
    pub fn render(&mut self, delta: i32) -> Option<Rendered> {
        let (ch, pg) = self.step(delta)?;
        let (w, h) = (self.w, self.h);
        let mut canvas = Canvas::new(w, h);
        self.laid(ch);
        let laid = &self.cache.last().unwrap().1;
        let geom = draw_page(&mut canvas, &mut self.fonts, &laid.pages[pg]);
        if delta == 0 {
            self.geom = Some(geom.clone());
        }
        Some(Rendered { canvas, geom })
    }

    /// Draw a page as RGBA (4 bytes a pixel, rows top to bottom) in the theme's colours.
    pub fn render_rgba(&mut self, delta: i32, theme: Theme) -> Option<Vec<u8>> {
        let r = self.render(delta)?;
        Some(to_rgba(&r, theme))
    }

    /// Where the current page sits in the whole book, 0.0 to 1.0, weighted by chapter length.
    pub fn progress(&mut self) -> f32 {
        let (page, pages) = self.page_in_chapter();
        if self.weights.is_none() {
            self.weights = Some(self.book.chapter_weights());
        }
        let w = self.weights.as_ref().unwrap();
        let total: f32 = w.iter().sum();
        if total <= 0.0 {
            return 0.0;
        }
        let before: f32 = w[..self.chapter.min(w.len())].iter().sum();
        let within = if pages > 1 { page as f32 / pages as f32 } else { 0.0 };
        ((before + within * w.get(self.chapter).copied().unwrap_or(0.0)) / total).clamp(0.0, 1.0)
    }

    /// Go to a fraction (0.0-1.0) of the way through the book, as a progress slider would.
    pub fn goto_progress(&mut self, f: f32) {
        if self.weights.is_none() {
            self.weights = Some(self.book.chapter_weights());
        }
        let w = self.weights.clone().unwrap();
        let total: f32 = w.iter().sum();
        let mut target = f.clamp(0.0, 1.0) * total;
        for (ch, cw) in w.iter().enumerate() {
            if target <= *cw || ch + 1 == w.len() {
                let frac = if *cw > 0.0 { (target / cw).clamp(0.0, 1.0) } else { 0.0 };
                self.goto(Position { chapter: ch, offset: 0 });
                let n = self.laid(ch).pages.len();
                let pg = ((frac * n as f32) as usize).min(n - 1);
                let offset = self.laid(ch).pages[pg].start;
                self.goto(Position { chapter: ch, offset });
                return;
            }
            target -= cw;
        }
    }

    fn current_geom(&mut self) -> PageGeom {
        if self.geom.is_none() {
            self.render(0);
        }
        self.geom.clone().unwrap_or_default()
    }

    /// What a tap at (x, y) on the current page hits.
    pub fn tap(&mut self, x: i32, y: i32) -> Tap {
        let geom = self.current_geom();
        let Some(&(_, idx)) = geom.links.iter().find(|(r, _)| r.contains(x, y)) else { return Tap::None };
        let ch = self.chapter;
        let Some(href) = self.laid(ch).links.get(idx as usize).cloned() else { return Tap::None };
        let Some((target, frag)) = self.book.resolve_link(ch, &href) else { return Tap::External(href) };
        let doc = self.book.chapter_doc(target);
        let offset = match frag.as_ref().map(|f| doc.anchors.get(f).copied()) {
            Some(Some(o)) => o,
            Some(None) if target == ch => return Tap::None, // a part this copy doesn't have
            _ => 0,
        };
        let position = Position { chapter: target, offset };
        let block = doc.blocks.iter().filter(|b| b.kind != BlockKind::Image).find(|b| b.start + b.chars() > offset);
        let text: String = block.map(|b| b.runs.iter().map(|r| r.text.as_str()).collect()).unwrap_or_default();
        if frag.is_some() && !text.is_empty() && text.chars().count() < 700 {
            return Tap::Note { text, target: position };
        }
        Tap::Jump(position)
    }

    /// The word under (x, y) on the current page.
    pub fn word_at(&mut self, x: i32, y: i32) -> Option<Word> {
        let geom = self.current_geom();
        let (rect, offset, text) = geom.words.iter().find(|(r, _, _)| r.contains(x, y))?.clone();
        let text = text.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
        (!text.is_empty()).then(|| Word { text, position: Position { chapter: self.chapter, offset }, rect })
    }

    /// The first words of the current page (a label for a bookmark).
    pub fn page_snippet(&mut self, words: usize) -> String {
        let geom = self.current_geom();
        geom.words.iter().take(words).map(|w| w.2.as_str()).collect::<Vec<_>>().join(" ")
    }

    /// Whether `pos` falls on the current page (to show a bookmark ribbon).
    pub fn on_current_page(&mut self, pos: Position) -> bool {
        self.settle();
        if pos.chapter != self.chapter {
            return false;
        }
        let (ch, pg) = (self.chapter, self.page);
        let laid = self.laid(ch);
        let start = laid.pages[pg].start;
        let end = laid.pages.get(pg + 1).map_or(usize::MAX, |p| p.start);
        pos.offset >= start && pos.offset < end
    }

    /// Case-insensitive search of the whole book.
    pub fn search(&mut self, query: &str, limit: usize) -> Vec<SearchHit> {
        search_book(&mut self.book, query, limit)
    }
}

/// Case-insensitive search through every chapter of a book.
pub fn search_book(book: &mut Book, q: &str, limit: usize) -> Vec<SearchHit> {
    let mut out = vec![];
    let qc: Vec<char> = q.to_lowercase().chars().collect();
    let qn = qc.len();
    if qn == 0 {
        return out;
    }
    for ch in 0..book.spine.len() {
        let blocks = book.chapter_blocks(ch);
        let title = book.chapter_title(ch);
        for b in blocks.iter().filter(|b| b.kind != BlockKind::Image) {
            let text: String = b.runs.iter().map(|r| r.text.as_str()).collect();
            let chars: Vec<char> = text.chars().collect();
            let lower: Vec<char> = text.to_lowercase().chars().collect();
            if lower.len() != chars.len() {
                continue; // rare case-mapping length change: skip rather than misplace
            }
            let mut i = 0;
            while i + qn <= lower.len() {
                if lower[i..i + qn] == qc[..] {
                    let s = i.saturating_sub(30);
                    let e = (i + qn + 40).min(chars.len());
                    let snippet = format!("{}{}{}", if s > 0 { "…" } else { "" },
                                          chars[s..e].iter().collect::<String>(), if e < chars.len() { "…" } else { "" });
                    out.push(SearchHit { snippet, chapter_title: title.clone(), position: Position { chapter: ch, offset: b.start + i } });
                    if out.len() >= limit {
                        return out;
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

/// Colour a grayscale page: text and lines between ink and paper; pictures kept natural if asked.
pub fn to_rgba(r: &Rendered, theme: Theme) -> Vec<u8> {
    let c = &r.canvas;
    let mut lut = [[0u8; 4]; 256];
    for (v, e) in lut.iter_mut().enumerate() {
        for k in 0..3 {
            let (ink, paper) = (theme.ink[k] as i32, theme.paper[k] as i32);
            e[k] = (ink + (paper - ink) * v as i32 / 255) as u8;
        }
        e[3] = 255;
    }
    let mut out = Vec::with_capacity((c.w * c.h * 4) as usize);
    for v in &c.buf {
        out.extend_from_slice(&lut[*v as usize]);
    }
    if theme.natural_images {
        for im in &r.geom.images {
            for y in im.y0.max(0)..im.y1.min(c.h) {
                for x in im.x0.max(0)..im.x1.min(c.w) {
                    let i = (y * c.w + x) as usize;
                    let v = c.buf[i];
                    out[i * 4..i * 4 + 3].copy_from_slice(&[v, v, v]);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/sample.epub");
        let mut s = Session::open(path, crate::test_fonts()).expect("open sample");
        s.set_viewport(600, 800);
        s
    }

    #[test]
    fn turns_pages_across_chapters_and_back() {
        let mut s = sample();
        assert!(s.chapter_count() > 1);
        assert!(!s.turn(-1), "no page before the first");
        let mut seen = vec![s.position()];
        while s.turn(1) {
            let p = s.position();
            assert!(p > *seen.last().unwrap(), "{p:?} after {seen:?}");
            seen.push(p);
        }
        assert_eq!(seen.last().unwrap().chapter, s.chapter_count() - 1);
        for p in seen.iter().rev().skip(1) {
            assert!(s.turn(-1));
            assert_eq!(s.position(), *p);
        }
        assert!(!s.turn(-1));
    }

    #[test]
    fn position_survives_relayout() {
        let mut s = sample();
        for _ in 0..3 {
            s.turn(1);
        }
        let before = s.position();
        s.set_style(Style { font_size: 28, ..Style::default() });
        let p = s.position();
        assert_eq!(p.chapter, before.chapter);
        assert!(p.offset <= before.offset, "the page shown still contains the old page's start");
        s.set_style(Style::default());
        s.set_viewport(1080, 1920);
        assert!(s.position().offset <= before.offset);
    }

    #[test]
    fn neighbours_render_without_moving() {
        let mut s = sample();
        s.turn(1);
        let here = s.position();
        assert!(s.render(-1).is_some());
        assert!(s.render(1).is_some());
        assert_eq!(s.position(), here);
        let rgba = s.render_rgba(0, Theme { paper: [20, 20, 20], ink: [230, 230, 230], natural_images: true }).unwrap();
        assert_eq!(rgba.len(), 600 * 800 * 4);
        assert_eq!(&rgba[..4], &[20, 20, 20, 255], "corner is paper");
        assert!(s.render(-100).is_none());
    }

    #[test]
    fn progress_grows_and_slider_lands_near() {
        let mut s = sample();
        let mut last = s.progress();
        assert_eq!(last, 0.0);
        while s.turn(1) {
            let p = s.progress();
            assert!(p >= last, "{p} < {last}");
            last = p;
        }
        assert!(last > 0.5);
        s.goto_progress(0.5);
        let p = s.progress();
        assert!((0.25..=0.75).contains(&p), "{p}");
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut s = sample();
        let word = s.page_snippet(1);
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        assert!(!word.is_empty());
        let hits = s.search(&word.to_uppercase(), 10);
        assert!(!hits.is_empty(), "no hits for {word}");
        s.goto(hits[0].position);
        assert!(s.on_current_page(hits[0].position));
    }
}
