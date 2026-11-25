//! Page reader for fixed-layout documents: PDF (MuPDF's `mutool`), DjVu (DjVuLibre's `ddjvu`) and
//! comics (CBZ zip / CBR via `bsdtar`). Pages are rendered as images in a background thread,
//! white margins are trimmed, and either the whole page fits the screen or ("fit width") the page
//! is shown at full width in screen-sized slices.

use crate::canvas::*;
use crate::image::{self, Gray};
use crate::input::{Gesture, Kind};
use crate::sys::Sys;
use crate::ui::*;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

pub const PAGE_EXT: &[&str] = &["pdf", "djvu", "djv", "cbz", "cbr"];

#[derive(Clone)]
enum Kind2 {
    Pdf,
    Djvu,
    Cbz(Vec<String>),
    Cbr(Vec<String>),
}

#[derive(Clone)]
pub struct PageDoc {
    path: String,
    kind: Kind2,
    pub pages: usize,
    pub title: String,
}

fn out(cmd: &[&str]) -> Result<Vec<u8>, String> {
    let o = Command::new(cmd[0]).args(&cmd[1..]).stdin(Stdio::null()).output()
        .map_err(|e| format!("{} is not available: {e}", cmd[0]))?;
    if o.status.success() && !o.stdout.is_empty() {
        Ok(o.stdout)
    } else {
        let e = String::from_utf8_lossy(&o.stderr);
        Err(e.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string())
    }
}

/// Image entries of an archive, in reading order ("page2" before "page10").
fn image_names(mut names: Vec<String>) -> Vec<String> {
    names.retain(|n| {
        let l = n.to_lowercase();
        !l.starts_with("__macosx") && !l.contains("/.") && [".jpg", ".jpeg", ".png"].iter().any(|e| l.ends_with(e))
    });
    let key = |s: &str| -> Vec<(String, u64)> {
        // split into text/number chunks for a natural sort
        let mut v = vec![];
        let mut text = String::new();
        let mut num = String::new();
        for c in s.to_lowercase().chars() {
            if c.is_ascii_digit() {
                num.push(c);
            } else {
                if !num.is_empty() {
                    v.push((std::mem::take(&mut text), num.parse().unwrap_or(0)));
                    num.clear();
                }
                text.push(c);
            }
        }
        v.push((text, num.parse().unwrap_or(0)));
        v
    };
    names.sort_by_key(|n| key(n));
    names
}

impl PageDoc {
    /// Comics are pictures: shown as they are, without trimming "margins".
    pub fn is_comic(&self) -> bool {
        matches!(self.kind, Kind2::Cbz(_) | Kind2::Cbr(_))
    }

    pub fn open(path: &str) -> Result<PageDoc, String> {
        let lower = path.to_lowercase();
        let title = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let title = crate::apps::book_title(&format!("{title}.x"));
        let (kind, pages) = if lower.ends_with(".pdf") {
            let info = String::from_utf8_lossy(&out(&["mutool", "show", path, "trailer/Root/Pages/Count"])?).into_owned();
            let n = info.trim().parse::<usize>().map_err(|_| format!("Couldn't read this PDF: {}", info.trim()))?;
            (Kind2::Pdf, n)
        } else if lower.ends_with(".djvu") || lower.ends_with(".djv") {
            let n = String::from_utf8_lossy(&out(&["djvused", "-e", "n", path])?).trim().parse::<usize>()
                .map_err(|_| "Couldn't read this DjVu file".to_string())?;
            (Kind2::Djvu, n)
        } else if lower.ends_with(".cbz") {
            let names = image_names(crate::epub::Zip::open(path)?.names());
            let n = names.len();
            (Kind2::Cbz(names), n)
        } else if lower.ends_with(".cbr") {
            let list = String::from_utf8_lossy(&out(&["bsdtar", "-tf", path])?).into_owned();
            let names = image_names(list.lines().map(String::from).collect());
            let n = names.len();
            (Kind2::Cbr(names), n)
        } else {
            return Err("Not a page document".into());
        };
        if pages == 0 {
            return Err("This file has no pages.".into());
        }
        Ok(PageDoc { path: path.into(), kind, pages, title })
    }

    /// Render page `i` (0-based) to fit within w×h.
    pub fn render(&self, i: usize, w: i32, h: i32) -> Result<Gray, String> {
        let (ws, hs, page) = (w.to_string(), h.to_string(), (i + 1).to_string());
        let bytes = match &self.kind {
            Kind2::Pdf => out(&["mutool", "draw", "-q", "-c", "gray", "-w", &ws, "-h", &hs, "-F", "pgm", "-o", "-", &self.path, &page])?,
            Kind2::Djvu => out(&["ddjvu", "-format=pgm", &format!("-page={page}"), &format!("-size={w}x{h}"), &self.path, "-"])?,
            Kind2::Cbz(names) => crate::epub::Zip::open(&self.path)?.read(&names[i]).ok_or("missing page image")?,
            Kind2::Cbr(names) => out(&["bsdtar", "-xOf", &self.path, &names[i]])?,
        };
        let g = image::decode(&bytes)?;
        Ok(if g.w > w || g.h > h { image::fit(&g, w, h) } else { g })
    }

    /// Table of contents: (title, page index). PDFs only.
    pub fn outline(&self) -> Vec<(String, usize)> {
        let Kind2::Pdf = self.kind else { return vec![] };
        let Ok(o) = out(&["mutool", "show", &self.path, "outline"]) else { return vec![] };
        let mut v = vec![];
        for line in String::from_utf8_lossy(&o).lines() {
            let depth = line.chars().take_while(|c| *c == '\t' || *c == '|' || *c == '+' || *c == '-' || *c == ' ').filter(|c| *c == '\t').count();
            let title = match (line.find('"'), line.rfind('"')) {
                (Some(a), Some(b)) if b > a => line[a + 1..b].to_string(),
                _ => continue,
            };
            let page = line.rsplit_once("#page=").or_else(|| line.rsplit_once('#'))
                .and_then(|(_, p)| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse::<usize>().ok());
            if let Some(p) = page.filter(|p| *p >= 1) {
                v.push((format!("{}{}", "   ".repeat(depth.saturating_sub(1).min(3)), title), p - 1));
            }
        }
        v
    }
}

/// First page as a small picture (library covers).
pub fn cover(path: &str, w: i32, h: i32) -> Option<Gray> {
    let d = PageDoc::open(path).ok()?;
    d.render(0, w * 2, h * 2).ok().map(|g| image::fit(&if d.is_comic() { g } else { image::trim(&g) }, w, h))
}

// --- the reader app ------------------------------------------------------------------------------
const FOOTER: i32 = 34;
const ID_PREV: u32 = 1;
const ID_NEXT: u32 = 2;
const ID_BAR: u32 = 3;
const ID_ZOOM: u32 = 4;
const ID_CONTENTS: u32 = 5;
const ID_GOTO: u32 = 6;
const T_GOTO: u32 = 1;

type Slot = Arc<Mutex<Option<(usize, Result<Gray, String>)>>>;

pub struct PageReader {
    doc: Arc<PageDoc>,
    page: usize,
    slice: usize,
    fit_width: bool,
    /// trimmed pages scaled for the current mode, by page index (a few kept)
    ready: HashMap<usize, Gray>,
    error: Option<String>,
    job: Option<(usize, Slot)>,
    area: (i32, i32),
    toolbar: bool,
    pick: Arc<Mutex<Option<(usize, usize)>>>,
    want_last_slice: bool,
}

impl PageReader {
    pub fn open(path: &str, sys: &Sys) -> Result<PageReader, String> {
        let doc = PageDoc::open(path)?;
        let (page, slice) = crate::reader::load_positions(sys).get(path).copied().unwrap_or((0, 0));
        let page = page.min(doc.pages - 1);
        let fit_width = sys.config.get("pages_fit") == "width";
        Ok(PageReader { doc: Arc::new(doc), page, slice, fit_width, ready: HashMap::new(), error: None, job: None,
                        area: (600, 800), toolbar: false, pick: Arc::new(Mutex::new(None)), want_last_slice: false })
    }

    fn content_h(&self) -> i32 {
        self.area.1 - FOOTER
    }

    /// Start rendering `page` in the background (if it isn't ready or already being rendered).
    fn request(&mut self, page: usize) {
        if page >= self.doc.pages || self.ready.contains_key(&page) || self.job.as_ref().map_or(false, |j| j.0 == page) {
            return;
        }
        if self.job.is_some() {
            return; // one at a time; poll() asks again
        }
        let (doc, (w, h), fit_width) = (self.doc.clone(), (self.area.0, self.content_h()), self.fit_width);
        let slot: Slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            // render bigger than the screen so trimming margins doesn't lose sharpness
            let r = doc.render(page, w * 2, h * 2).map(|g| {
                let t = if doc.is_comic() { g } else { image::trim(&g) };
                if fit_width {
                    let s = w as f32 / t.w as f32;
                    image::resize(&t, w, ((t.h as f32 * s) as i32).max(1))
                } else {
                    image::fit(&t, w, h)
                }
            });
            *s2.lock().unwrap() = Some((page, r));
        });
        self.job = Some((page, slot));
    }

    fn slices(&self, page: usize) -> usize {
        let h = self.content_h();
        // windows of height h, each starting h-40 below the previous (40 px overlap)
        self.ready.get(&page).map_or(1, |g| if g.h <= h { 1 } else { (((g.h - h) + (h - 41)) / (h - 40) + 1) as usize })
    }

    fn save(&self, sys: &Sys) {
        if sys.sim && self.doc.path.is_empty() {
            return;
        }
        crate::reader::save_position(sys, &self.doc.path, self.page, self.slice);
        let pct = ((self.page + 1) * 100 / self.doc.pages) as u32;
        crate::covers::save_progress(sys, &self.doc.path, pct, &self.doc.title, "");
        let _ = std::fs::write(format!("{}/kos/last-book.txt", sys.us),
                               format!("{}\t{}\t\t{}\n", self.doc.path, self.doc.title.replace('\t', " "), pct));
    }

    fn go(&mut self, page: usize, slice: usize, sys: &Sys) -> Cmd {
        self.page = page.min(self.doc.pages - 1);
        self.slice = slice;
        self.error = None;
        self.toolbar = false;
        self.save(sys);
        self.request(self.page);
        Cmd::Redraw
    }

    fn next(&mut self, sys: &Sys) -> Cmd {
        if self.slice + 1 < self.slices(self.page) {
            let s = self.slice + 1;
            return self.go(self.page, s, sys);
        }
        if self.page + 1 < self.doc.pages {
            let p = self.page + 1;
            return self.go(p, 0, sys);
        }
        Cmd::None
    }

    fn prev(&mut self, sys: &Sys) -> Cmd {
        if self.slice > 0 {
            let s = self.slice - 1;
            return self.go(self.page, s, sys);
        }
        if self.page > 0 {
            let p = self.page - 1;
            self.want_last_slice = true;
            let c = self.go(p, 0, sys);
            if self.ready.contains_key(&p) {
                self.slice = self.slices(p) - 1;
                self.want_last_slice = false;
            }
            return c;
        }
        Cmd::None
    }

    fn render_toolbar(&mut self, cx: &mut Ctx, a: Rect) {
        cx.hit(a, Hit::App(ID_BAR), Some(Hit::App(ID_BAR)), "close toolbar");
        let b = Btn::size(20);
        let top = Rect::new(a.x0, a.y0, a.x1, a.y0 + 64);
        cx.c.fill(top, WHITE);
        cx.c.hline(a.x0, a.x1, top.y1 - 2, 2, BLACK);
        cx.button(Rect::new(6, top.y0 + 6, 70, top.y1 - 8), "‹", Some(Hit::Back), b);
        cx.button(Rect::new(76, top.y0 + 6, 140, top.y1 - 8), "⌂", Some(Hit::Home), Btn { size: 24, ..b });
        let t = cx.ellipsize(&self.doc.title, Face::Bold, 20, a.w() - 160);
        cx.bold(152, top.cy() - 2, &t, 20, BLACK, Anchor::LM);
        let bot = Rect::new(a.x0, a.y1 - 118, a.x1, a.y1);
        cx.c.fill(bot, WHITE);
        cx.c.hline(a.x0, a.x1, bot.y0, 2, BLACK);
        cx.bold(a.x0 + 16, bot.y0 + 20, &format!("Page {} of {}", self.page + 1, self.doc.pages), 16, BLACK, Anchor::LM);
        let mode = if self.fit_width { "fit width" } else { "whole page" };
        cx.text(a.x1 - 16, bot.y0 + 20, mode, 15, DARK, Anchor::RM);
        let bar = Rect::new(a.x0 + 16, bot.y0 + 36, a.x1 - 16, bot.y0 + 44);
        cx.c.round_rect(bar, 4, WHITE, 1, BLACK);
        let w = (bar.w() - 2) * (self.page as i32 + 1) / self.doc.pages as i32;
        cx.c.fill(Rect::new(bar.x0 + 1, bar.y0 + 1, bar.x0 + 1 + w, bar.y1 - 1), BLACK);
        let zoom = if self.fit_width { "Whole page" } else { "Fit width" };
        let labels = [("Contents", ID_CONTENTS), ("Go to page", ID_GOTO), (zoom, ID_ZOOM)];
        let bw = (a.w() - 16 - 16) / 3;
        for (k, (l, id)) in labels.iter().enumerate() {
            let x = a.x0 + 8 + k as i32 * (bw + 8);
            cx.button(Rect::new(x, bot.y0 + 56, x + bw, bot.y1 - 10), l, Some(Hit::App(*id)), b);
        }
    }
}

impl App for PageReader {
    fn title(&self) -> String {
        self.doc.title.clone()
    }

    fn fullscreen(&self) -> bool {
        true
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        if self.area != (a.w(), a.h()) {
            self.area = (a.w(), a.h());
            self.ready.clear();
        }
        self.request(self.page);
        let ch = self.content_h();
        match (self.ready.get(&self.page), &self.error) {
            (Some(g), _) => {
                let slices = self.slices(self.page);
                let slice = self.slice.min(slices - 1);
                // slices overlap by 40 px so no line is cut in half without being seen whole
                let y_off = if slices == 1 { 0 } else { (slice as i32 * (ch - 40)).min(g.h - ch) };
                let (x, y) = (a.x0 + (a.w() - g.w) / 2, if slices == 1 { a.y0 + (ch - g.h) / 2 } else { a.y0 - y_off });
                let rows = g.h.min(y_off + ch) - y_off;
                if slices == 1 {
                    cx.c.blit(&g.px, g.w, g.h, x, y);
                } else {
                    let start = (y_off * g.w) as usize;
                    cx.c.blit(&g.px[start..start + (rows * g.w) as usize], g.w, rows, x, a.y0);
                }
            }
            (None, Some(e)) => {
                cx.bold(a.cx(), a.y0 + ch / 2 - 20, "Couldn't show this page", 22, BLACK, Anchor::MM);
                let e = cx.ellipsize(e, Face::Sans, 16, a.w() - 40);
                cx.text(a.cx(), a.y0 + ch / 2 + 16, &e, 16, DARK, Anchor::MM);
            }
            (None, None) => {
                cx.text(a.cx(), a.y0 + ch / 2, &format!("Loading page {}…", self.page + 1), 20, DARK, Anchor::MM);
            }
        }
        // footer
        let fy = a.y1 - FOOTER / 2;
        cx.c.hline(a.x0 + 16, a.x1 - 16, a.y1 - FOOTER + 2, 1, LIGHT);
        let slices = self.slices(self.page);
        let part = if slices > 1 { format!("  (part {} of {})", self.slice.min(slices - 1) + 1, slices) } else { String::new() };
        cx.text(a.x0 + 16, fy, &format!("{} / {}{part}", self.page + 1, self.doc.pages), 15, DARK, Anchor::LM);
        cx.text(a.x1 - 16, fy, &format!("{}%", (self.page + 1) * 100 / self.doc.pages), 15, DARK, Anchor::RM);
        if self.toolbar {
            self.render_toolbar(cx, a);
            return;
        }
        let third = a.w() / 3;
        cx.hit(a, Hit::App(ID_NEXT), Some(Hit::App(ID_BAR)), "next page");
        cx.hit(Rect::new(a.x0, a.y0, a.x0 + third, a.y1), Hit::App(ID_PREV), Some(Hit::App(ID_BAR)), "previous page");
        cx.hit(Rect::new(a.x0, a.y0, a.x1, a.y0 + a.h() / 7), Hit::App(ID_BAR), Some(Hit::App(ID_BAR)), "toolbar");
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn poll(&mut self, _sys: &mut Sys) -> Cmd {
        let done = self.job.as_ref().and_then(|(_, s)| s.lock().unwrap().take());
        let Some((page, r)) = done else { return Cmd::None };
        self.job = None;
        let shown = page == self.page;
        match r {
            Ok(g) => {
                self.ready.insert(page, g);
                if shown && self.want_last_slice {
                    self.slice = self.slices(page) - 1;
                    self.want_last_slice = false;
                }
            }
            Err(e) if shown => self.error = Some(e),
            Err(_) => {}
        }
        // keep only the pages around the current one, then prefetch the next
        let cur = self.page;
        self.ready.retain(|p, _| *p + 1 >= cur && *p <= cur + 1);
        if !self.ready.contains_key(&cur) {
            self.request(cur);
        } else {
            self.request(cur + 1);
        }
        if shown { Cmd::Redraw } else { Cmd::None }
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        match id {
            ID_NEXT => self.next(sys),
            ID_PREV => self.prev(sys),
            ID_BAR => {
                self.toolbar = !self.toolbar;
                if self.toolbar { Cmd::Fast } else { Cmd::Redraw }
            }
            ID_ZOOM => {
                self.fit_width = !self.fit_width;
                sys.config.set("pages_fit", if self.fit_width { "width" } else { "page" });
                self.ready.clear();
                self.job = None;
                self.slice = 0;
                self.toolbar = false;
                self.request(self.page);
                Cmd::Full
            }
            ID_CONTENTS => {
                self.toolbar = false;
                let items: Vec<(String, String, usize, usize)> = self.doc.outline().into_iter()
                    .map(|(t, p)| (t, format!("page {}", p + 1), p, 0)).collect();
                let mark = items.iter().rposition(|i| i.2 <= self.page);
                Cmd::Push(Box::new(crate::reader::JumpList::new(format!("Contents · {}", self.doc.title), items, mark,
                                                               self.pick.clone(), "This document has no table of contents")))
            }
            ID_GOTO => {
                self.toolbar = false;
                Cmd::Dialog(Dialog::input(T_GOTO, "Go to page", "", "Go").with_message(&format!("1 to {}", self.doc.pages)))
            }
            _ => Cmd::None,
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if tag == T_GOTO && choice != CANCEL {
            if let Ok(n) = text.trim().parse::<usize>() {
                if n >= 1 {
                    return self.go(n - 1, 0, sys);
                }
            }
            return Cmd::Toast("Type a page number".into());
        }
        Cmd::None
    }

    fn gesture(&mut self, g: &Gesture, sys: &mut Sys) -> Cmd {
        match g.kind {
            Kind::SwipeLeft | Kind::SwipeUp => self.next(sys),
            Kind::SwipeRight | Kind::SwipeDown => {
                let c = self.prev(sys);
                if let Cmd::None = c { Cmd::Fast } else { c }
            }
            _ => Cmd::None,
        }
    }

    fn on_show(&mut self, sys: &mut Sys) {
        let picked = self.pick.lock().unwrap().take();
        if let Some((p, _)) = picked {
            self.go(p, 0, sys);
        }
    }

    fn on_hide(&mut self, sys: &mut Sys) {
        self.save(sys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_page_order() {
        let n = image_names(vec!["c/page10.jpg".into(), "c/page2.jpg".into(), "c/page1.png".into(), "__MACOSX/x.jpg".into(),
                                 "c/notes.txt".into()]);
        assert_eq!(n, vec!["c/page1.png", "c/page2.jpg", "c/page10.jpg"]);
    }
}
