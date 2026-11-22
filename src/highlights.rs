//! Highlights (saved from the reader or imported from the Kindle's "My Clippings.txt"),
//! dictionary lookups, and reading statistics.

use crate::canvas::*;
use crate::getbooks::http_get;
use crate::input::Gesture;
use crate::json;
use crate::sys::{self, Sys};
use crate::ui::*;
use std::collections::BTreeMap;

// --- highlights store ------------------------------------------------------------------------------
#[derive(Clone)]
pub struct Highlight {
    pub date: String,
    pub book: String,
    pub place: String,
    pub text: String,
}

fn hl_path(sys: &Sys) -> String {
    format!("{}/kos/highlights.txt", sys.us)
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ").trim().to_string()
}

pub fn load(sys: &Sys) -> Vec<Highlight> {
    std::fs::read_to_string(hl_path(sys)).unwrap_or_default().lines().filter_map(|l| {
        let p: Vec<&str> = l.splitn(4, '\t').collect();
        (p.len() == 4).then(|| Highlight { date: p[0].into(), book: p[1].into(), place: p[2].into(), text: p[3].into() })
    }).collect()
}

fn save_all(sys: &Sys, v: &[Highlight]) {
    let body: String = v.iter().map(|h| format!("{}\t{}\t{}\t{}\n", h.date, clean(&h.book), clean(&h.place), clean(&h.text))).collect();
    let _ = std::fs::create_dir_all(format!("{}/kos", sys.us));
    let _ = std::fs::write(hl_path(sys), body);
}

pub fn add(sys: &Sys, book: &str, place: &str, text: &str) {
    let mut v = load(sys);
    if v.iter().any(|h| h.text == clean(text) && h.book == clean(book)) {
        return;
    }
    v.push(Highlight { date: sys::clock("%Y-%m-%d"), book: clean(book), place: clean(place), text: clean(text) });
    save_all(sys, &v);
}

/// Parse the Kindle's "My Clippings.txt" (entries separated by "=========="). Returns how many were new.
pub fn import_clippings(sys: &Sys) -> Result<usize, String> {
    let path = format!("{}/documents/My Clippings.txt", sys.us);
    let raw = std::fs::read(&path).map_err(|_| "No “My Clippings.txt” found in documents/".to_string())?;
    let text = String::from_utf8_lossy(&raw).replace('\u{feff}', "");
    let mut v = load(sys);
    let mut added = 0;
    for entry in text.split("==========") {
        let lines: Vec<&str> = entry.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if lines.len() < 3 {
            continue;
        }
        let (book, meta, body) = (lines[0], lines[1], lines[2..].join(" "));
        if meta.contains("Bookmark") {
            continue;
        }
        let place = meta.trim_start_matches("- ").split('|').next().unwrap_or("").trim().to_string();
        let date = meta.split("Added on").nth(1).map(|d| d.trim().to_string()).unwrap_or_default();
        if v.iter().any(|h| h.text == clean(&body) && h.book == clean(book)) {
            continue;
        }
        v.push(Highlight { date, book: clean(book), place: clean(&place), text: clean(&body) });
        added += 1;
    }
    save_all(sys, &v);
    Ok(added)
}

// --- dictionary -------------------------------------------------------------------------------------
/// Look a word up (English, dictionaryapi.dev). Returns a short multi-line definition.
pub fn define(word: &str) -> Result<String, String> {
    let w: String = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'').to_lowercase();
    if w.is_empty() {
        return Err("No word selected".into());
    }
    let body = http_get(&format!("https://api.dictionaryapi.dev/api/v2/entries/en/{w}"))
        .map_err(|e| if e.contains("404") || e.contains("error: 404") { format!("No definition found for “{w}”") } else { e })?;
    let j = json::parse(&body).map_err(|e| e.to_string())?;
    let entry = j.arr().first().ok_or_else(|| format!("No definition found for “{w}”"))?;
    let mut out = String::new();
    if let Some(p) = entry.get("phonetic").str() {
        out.push_str(p);
        out.push('\n');
    }
    let mut n = 0;
    for m in entry.get("meanings").arr() {
        let pos = m.get("partOfSpeech").str().unwrap_or("");
        for d in m.get("definitions").arr().iter().take(2) {
            if let Some(def) = d.get("definition").str() {
                n += 1;
                out.push_str(&format!("{n}. ({pos}) {def}\n"));
            }
            if n >= 4 {
                break;
            }
        }
        if n >= 4 {
            break;
        }
    }
    if n == 0 {
        return Err(format!("No definition found for “{w}”"));
    }
    Ok(out.trim().to_string())
}

// --- reading stats -----------------------------------------------------------------------------------
fn stats_path(sys: &Sys) -> String {
    format!("{}/kos/stats.txt", sys.us)
}

/// day -> (seconds, pages)
pub fn load_stats(sys: &Sys) -> BTreeMap<String, (u64, u64)> {
    let mut m = BTreeMap::new();
    for l in std::fs::read_to_string(stats_path(sys)).unwrap_or_default().lines() {
        let p: Vec<&str> = l.split('\t').collect();
        if p.len() >= 3 {
            m.insert(p[0].to_string(), (p[1].parse().unwrap_or(0), p[2].parse().unwrap_or(0)));
        }
    }
    m
}

/// Record reading activity for today (called on page turns).
pub fn record(sys: &Sys, seconds: u64, pages: u64) {
    let mut m = load_stats(sys);
    let e = m.entry(sys::clock("%Y-%m-%d")).or_insert((0, 0));
    e.0 += seconds;
    e.1 += pages;
    let body: String = m.iter().map(|(d, (s, p))| format!("{d}\t{s}\t{p}\n")).collect();
    let _ = std::fs::create_dir_all(format!("{}/kos", sys.us));
    let _ = std::fs::write(stats_path(sys), body);
}

pub fn mark_finished(sys: &Sys, title: &str) {
    let path = format!("{}/kos/finished.txt", sys.us);
    let cur = std::fs::read_to_string(&path).unwrap_or_default();
    if !cur.lines().any(|l| l.split('\t').nth(1) == Some(&clean(title))) {
        let _ = std::fs::write(&path, format!("{cur}{}\t{}\n", sys::clock("%Y-%m-%d"), clean(title)));
    }
}

fn day_offset(days_ago: i64) -> String {
    let t = unsafe { libc::time(std::ptr::null_mut()) } as i64 - days_ago * 86400;
    sys::format_time(t, "%Y-%m-%d")
}

// --- Highlights screen ------------------------------------------------------------------------------
pub struct HighlightsApp {
    items: Vec<Highlight>,
    pager: Pager,
    sel: usize,
}

const ID_IMPORT: u32 = 1_000_001;
const T_DELETE: u32 = 1;

impl HighlightsApp {
    pub fn new() -> HighlightsApp {
        HighlightsApp { items: vec![], pager: Pager::new(110), sel: 0 }
    }
}

impl App for HighlightsApp {
    fn title(&self) -> String {
        format!("Highlights ({})", self.items.len())
    }

    fn on_show(&mut self, sys: &mut Sys) {
        let mut v = load(sys);
        v.reverse(); // newest first
        self.items = v;
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x1 - 12, a.y0 + 58), "Import Kindle “My Clippings”",
                  Some(Hit::App(ID_IMPORT)), Btn::size(19));
        let items = &self.items;
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 70, a.x1, a.y1), items.len(),
                          "No highlights yet. Long-press a word while reading.", |cx, r, i| {
            let h = &items[i];
            let lines = cx.f.wrap(&format!("“{}”", h.text), Face::Sans, cx.sz(17), r.w() - 100);
            for (k, l) in lines.iter().take(3).enumerate() {
                let l = if k == 2 && lines.len() > 3 { format!("{l}…") } else { l.clone() };
                cx.text(r.x0 + 20, r.y0 + 10 + k as i32 * 24, &l, 17, BLACK, Anchor::LT);
            }
            let src = cx.ellipsize(&format!("{}  ·  {}", h.book, h.date), Face::Sans, 14, r.w() - 100);
            cx.text(r.x0 + 20, r.y1 - 24, &src, 14, DARK, Anchor::LT);
            cx.hit(r, Hit::App(i as u32), Some(Hit::App(0x8000_0000 | i as u32)), &h.text);
            more_button(cx, r, 0x8000_0000 | i as u32);
        });
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        match id {
            ID_IMPORT => match import_clippings(sys) {
                Ok(n) => {
                    self.on_show(sys);
                    Cmd::Toast(format!("Imported {n} new highlight(s)"))
                }
                Err(e) => Cmd::Dialog(Dialog::message("Import", &e)),
            },
            i if i & 0x8000_0000 != 0 => {
                self.sel = (i & 0x7fff_ffff) as usize;
                Cmd::Dialog(Dialog::confirm(T_DELETE, "Delete highlight?", "", "Delete"))
            }
            i => match self.items.get(i as usize) {
                Some(h) => Cmd::Dialog(Dialog::message(&h.book.chars().take(40).collect::<String>(),
                                                       &format!("“{}”\n\n{}", h.text, h.place))),
                None => Cmd::None,
            },
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, _t: String, sys: &mut Sys) -> Cmd {
        if tag == T_DELETE && choice != CANCEL && self.sel < self.items.len() {
            let gone = self.items.remove(self.sel);
            let mut all = load(sys);
            all.retain(|h| !(h.text == gone.text && h.book == gone.book));
            save_all(sys, &all);
            return Cmd::Redraw;
        }
        Cmd::None
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}

// --- Stats screen -------------------------------------------------------------------------------------
pub struct StatsApp;

impl App for StatsApp {
    fn title(&self) -> String {
        "Reading stats".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        let m = load_stats(sys);
        let today = m.get(&day_offset(0)).copied().unwrap_or((0, 0));
        // streak: consecutive days (ending today or yesterday) with any pages read
        let mut streak = 0;
        let start = if today.1 > 0 { 0 } else { 1 };
        for d in start..3650 {
            if m.get(&day_offset(d)).map_or(false, |e| e.1 > 0) { streak += 1 } else { break }
        }
        let total_min: u64 = m.values().map(|e| e.0).sum::<u64>() / 60;
        let total_pages: u64 = m.values().map(|e| e.1).sum();
        let finished: Vec<String> = std::fs::read_to_string(format!("{}/kos/finished.txt", sys.us)).unwrap_or_default()
            .lines().filter_map(|l| l.split('\t').nth(1).map(String::from)).collect();
        let tiles = [
            (format!("{}", today.0 / 60), "min today"),
            (format!("{}", today.1), "pages today"),
            (format!("{streak}"), if streak == 1 { "day streak" } else { "days streak" }),
            (format!("{}", finished.len()), "books finished"),
        ];
        let tw = (a.w() - 24 - 3 * 10) / 4;
        for (i, (n, l)) in tiles.iter().enumerate() {
            let x = a.x0 + 12 + i as i32 * (tw + 10);
            let r = Rect::new(x, a.y0 + 16, x + tw, a.y0 + 116);
            cx.c.round_rect(r, 12, WHITE, 2, BLACK);
            cx.bold(r.cx(), r.y0 + 40, n, 36, BLACK, Anchor::MM);
            cx.text(r.cx(), r.y1 - 20, l, 14, DARK, Anchor::MM);
        }
        // last 14 days bar chart (minutes)
        let cy = a.y0 + 150;
        cx.bold(a.x0 + 20, cy, "Last 14 days (minutes)", 18, BLACK, Anchor::LT);
        let days: Vec<(String, u64)> = (0..14).rev().map(|d| {
            let k = day_offset(d);
            (k.clone(), m.get(&k).map_or(0, |e| e.0 / 60))
        }).collect();
        let max = days.iter().map(|d| d.1).max().unwrap_or(0).max(10);
        let (ch_top, ch_bot) = (cy + 40, cy + 260);
        let bw = (a.w() - 40) / 14;
        cx.c.hline(a.x0 + 20, a.x1 - 20, ch_bot, 2, BLACK);
        for (i, (k, mins)) in days.iter().enumerate() {
            let x = a.x0 + 20 + i as i32 * bw;
            let h = ((*mins as f32 / max as f32) * (ch_bot - ch_top) as f32) as i32;
            if h > 0 {
                cx.c.fill(Rect::new(x + 5, ch_bot - h, x + bw - 5, ch_bot), if i == 13 { BLACK } else { DARK });
            }
            if *mins > 0 {
                cx.text(x + bw / 2, ch_bot - h - 18, &mins.to_string(), 12, BLACK, Anchor::MM);
            }
            if i % 2 == 1 || i == 13 {
                cx.text(x + bw / 2, ch_bot + 14, &k[8..], 12, DARK, Anchor::MM);
            }
        }
        let y = ch_bot + 44;
        cx.text(a.x0 + 20, y, &format!("All time: {total_min} min · {total_pages} pages"), 17, BLACK, Anchor::LT);
        cx.bold(a.x0 + 20, y + 40, "Finished", 18, BLACK, Anchor::LT);
        if finished.is_empty() {
            cx.text(a.x0 + 20, y + 72, "Nothing yet — happy reading!", 16, DARK, Anchor::LT);
        }
        for (i, b) in finished.iter().rev().take(6).enumerate() {
            let t = cx.ellipsize(b, Face::Sans, 16, a.w() - 60);
            cx.text(a.x0 + 30, y + 72 + i as i32 * 26, &format!("• {t}"), 16, BLACK, Anchor::LT);
        }
    }
}
