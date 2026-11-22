//! Immediate-mode UI for e-ink: apps render into a canvas and register hit boxes that carry
//! action ids. Taps are matched against the last frame's boxes and dispatched to the app,
//! which returns a `Cmd` for the UI manager (push screen, dialog, redraw, quit...).

use crate::canvas::*;
use crate::fb::{Display, Update};
use crate::input::{Gesture, Kind};
use crate::sys::Sys;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

pub const HEADER_H: i32 = 56;
pub const PAGER_PREV: u32 = 0xFFFF_0001;
pub const PAGER_NEXT: u32 = 0xFFFF_0002;

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Char(char),
    Backspace,
    Enter,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Ctrl(char),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyBtn {
    Char(char),
    Backspace,
    Enter,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Shift,
    Sym,
    Ctrl,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Back,
    Home,
    App(u32),
    Key(KeyBtn),
    DialogButton(usize),
    DialogField,
    Swallow,
}

pub struct HitBox {
    pub r: Rect,
    pub tap: Hit,
    pub long: Option<Hit>,
    pub label: String,
}

pub enum Cmd {
    None,
    Redraw,
    Fast,
    Full,
    Push(Box<dyn App>),
    /// Resume a parked persistent app with this key, or create it.
    Resume(&'static str, fn(&mut Sys) -> Box<dyn App>),
    Pop,
    Dialog(Dialog),
    Toast(String),
    Quit(i32),
    Batch(Vec<Cmd>),
}

pub struct Ctx<'a> {
    pub c: &'a mut Canvas,
    pub f: &'a mut Fonts,
    pub hits: Vec<HitBox>,
    /// Interface text scale ("Larger menu text"); book pages draw their text directly and ignore it.
    pub scale: f32,
}

#[derive(Clone, Copy)]
pub struct Btn<'a> {
    pub size: u16,
    pub bold: bool,
    pub inverted: bool,
    pub disabled: bool,
    pub radius: i32,
    pub border: i32,
    pub sub: Option<&'a str>,
}

impl Default for Btn<'_> {
    fn default() -> Self {
        Btn { size: 22, bold: false, inverted: false, disabled: false, radius: 10, border: 2, sub: None }
    }
}

impl Btn<'_> {
    pub fn size(size: u16) -> Self {
        Btn { size, ..Default::default() }
    }
}

impl<'a> Ctx<'a> {
    pub fn w(&self) -> i32 {
        self.c.w
    }
    pub fn h(&self) -> i32 {
        self.c.h
    }

    pub fn sz(&self, size: u16) -> u16 {
        (size as f32 * self.scale).round() as u16
    }
    pub fn text(&mut self, x: i32, y: i32, s: &str, size: u16, color: u8, anchor: Anchor) -> i32 {
        let size = self.sz(size);
        self.c.text(self.f, x, y, s, Face::Sans, size, color, anchor)
    }
    pub fn bold(&mut self, x: i32, y: i32, s: &str, size: u16, color: u8, anchor: Anchor) -> i32 {
        let size = self.sz(size);
        self.c.text(self.f, x, y, s, Face::Bold, size, color, anchor)
    }
    pub fn mono(&mut self, x: i32, y: i32, s: &str, size: u16, color: u8) -> i32 {
        self.c.text(self.f, x, y, s, Face::Mono, size, color, Anchor::LT)
    }
    pub fn ellipsize(&mut self, s: &str, face: Face, size: u16, w: i32) -> String {
        let size = self.sz(size);
        self.f.ellipsize(s, face, size, w)
    }

    pub fn hit(&mut self, r: Rect, tap: Hit, long: Option<Hit>, label: &str) {
        self.hits.push(HitBox { r, tap, long, label: label.to_string() });
    }

    pub fn button(&mut self, r: Rect, label: &str, hit: Option<Hit>, mut o: Btn) {
        o.size = self.sz(o.size).min((r.h() as u16).saturating_sub(10).max(12));
        let (bg, mut fg) = if o.inverted { (BLACK, WHITE) } else { (WHITE, BLACK) };
        let border_color = if o.disabled { MID } else { BLACK };
        if o.disabled {
            fg = MID;
        }
        self.c.round_rect(r, o.radius, bg, o.border, border_color);
        let face = if o.bold { Face::Bold } else { Face::Sans };
        let lab = self.f.ellipsize(label, face, o.size, r.w() - 16);
        let cy = r.cy() - if o.sub.is_some() { 11 } else { 0 };
        self.c.text(self.f, r.cx(), cy, &lab, face, o.size, fg, Anchor::MM);
        if let Some(sub) = o.sub {
            let ss = o.size.saturating_sub(8).max(14);
            let s = self.f.ellipsize(sub, Face::Sans, ss, r.w() - 16);
            self.c.text(self.f, r.cx(), cy + o.size as i32 / 2 + 12, &s, Face::Sans, ss,
                        if o.inverted { WHITE } else { DARK }, Anchor::MM);
        }
        if let (Some(h), false) = (hit, o.disabled) {
            self.hit(r, h, None, label);
        }
    }

    pub fn hline(&mut self, x0: i32, x1: i32, y: i32, width: i32, v: u8) {
        self.c.hline(x0, x1, y, width, v);
    }
}

/// Paged list (e-ink friendly: no scrolling).
pub struct Pager {
    pub page: usize,
    pub row_h: i32,
    pages: usize,
}

impl Pager {
    pub fn new(row_h: i32) -> Pager {
        Pager { page: 0, row_h, pages: 1 }
    }

    /// Lays out rows; calls draw(ctx, rect, index) for each visible item.
    pub fn render(&mut self, cx: &mut Ctx, area: Rect, count: usize, empty: &str,
                  mut draw: impl FnMut(&mut Ctx, Rect, usize)) {
        let per = (((area.h() - 64) / self.row_h).max(1)) as usize;
        self.pages = ((count + per - 1) / per).max(1);
        self.page = self.page.min(self.pages - 1);
        if count == 0 {
            cx.text(area.cx(), area.y0 + 80, empty, 22, DARK, Anchor::MM);
        }
        let mut y = area.y0;
        for i in self.page * per..(self.page * per + per).min(count) {
            let r = Rect::new(area.x0, y, area.x1, y + self.row_h);
            draw(cx, r, i);
            cx.hline(area.x0 + 12, area.x1 - 12, y + self.row_h - 1, 1, LIGHT);
            y += self.row_h;
        }
        if self.pages > 1 {
            let by = area.y1 - 58;
            let first = self.page == 0;
            let last = self.page + 1 >= self.pages;
            cx.button(Rect::new(area.x0 + 12, by, area.x0 + 150, by + 50), "◀ Prev", Some(Hit::App(PAGER_PREV)),
                      Btn { size: 20, disabled: first, ..Default::default() });
            cx.text(area.cx(), by + 25, &format!("{} / {}", self.page + 1, self.pages), 20, BLACK, Anchor::MM);
            cx.button(Rect::new(area.x1 - 150, by, area.x1 - 12, by + 50), "Next ▶", Some(Hit::App(PAGER_NEXT)),
                      Btn { size: 20, disabled: last, ..Default::default() });
        }
    }

    /// Handle pager actions/swipes; returns true if consumed.
    pub fn action(&mut self, id: u32) -> bool {
        match id {
            PAGER_PREV => {
                self.page = self.page.saturating_sub(1);
                true
            }
            PAGER_NEXT => {
                if self.page + 1 < self.pages {
                    self.page += 1;
                }
                true
            }
            _ => false,
        }
    }

    pub fn swipe(&mut self, g: &Gesture) -> bool {
        match g.kind {
            Kind::SwipeLeft | Kind::SwipeUp => self.action(PAGER_NEXT),
            Kind::SwipeRight | Kind::SwipeDown => self.action(PAGER_PREV),
            _ => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Lower,
    Upper,
    Sym,
}

pub struct Keyboard {
    layout: Layout,
    pub ctrl: bool,
    pub extended: bool,
}

const ROWS_LOWER: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];
const ROWS_UPPER: [&str; 4] = ["!@#$%^&*()", "QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"];
const ROWS_SYM: [&str; 4] = ["1234567890", "-/:;()$&@\"", "[]{}#%^*+=", "_\\|~<>'`?!"];

impl Keyboard {
    pub fn new() -> Keyboard {
        Keyboard { layout: Layout::Lower, ctrl: false, extended: false }
    }

    pub fn height(&self) -> i32 {
        300 + if self.extended { 52 } else { 0 }
    }

    pub fn render(&self, cx: &mut Ctx, y0: i32) {
        let w = cx.w();
        let h = cx.h();
        cx.c.fill(Rect::new(0, y0, w, h), WHITE);
        cx.hline(0, w, y0, 2, BLACK);
        let mut y = y0 + 6;
        let kh = 54;
        let small = Btn { size: 18, radius: 6, ..Default::default() };
        if self.extended {
            let keys: [(&str, KeyBtn); 10] = [("Esc", KeyBtn::Esc), ("Tab", KeyBtn::Tab), ("Ctrl", KeyBtn::Ctrl),
                ("←", KeyBtn::Left), ("↓", KeyBtn::Down), ("↑", KeyBtn::Up), ("→", KeyBtn::Right),
                ("|", KeyBtn::Char('|')), ("~", KeyBtn::Char('~')), ("/", KeyBtn::Char('/'))];
            let kw = (w - 8) as f32 / keys.len() as f32;
            for (i, (lab, k)) in keys.iter().enumerate() {
                let r = Rect::new((4.0 + i as f32 * kw) as i32 + 2, y, (4.0 + (i + 1) as f32 * kw) as i32 - 2, y + 46);
                let inv = *k == KeyBtn::Ctrl && self.ctrl;
                cx.button(r, lab, Some(Hit::Key(*k)), Btn { inverted: inv, ..small });
            }
            y += 52;
        }
        let rows = match self.layout {
            Layout::Lower => ROWS_LOWER,
            Layout::Upper => ROWS_UPPER,
            Layout::Sym => ROWS_SYM,
        };
        let key = Btn { size: 24, radius: 6, ..Default::default() };
        for (ri, row) in rows.iter().enumerate() {
            let n = row.chars().count() as f32;
            let extra = if ri == 3 { 1.5 } else { 0.0 };
            let kw = (w - 8) as f32 / (n + 2.0 * extra).max(10.0);
            let mut x = 4.0 + ((w - 8) as f32 - kw * (n + 2.0 * extra)) / 2.0;
            if ri == 3 {
                let lab = if self.layout == Layout::Sym { "abc" } else { "⇧" };
                let r = Rect::new(x as i32 + 2, y, (x + kw * extra) as i32 - 2, y + kh - 4);
                cx.button(r, lab, Some(Hit::Key(KeyBtn::Shift)),
                          Btn { size: 22, inverted: self.layout == Layout::Upper, ..key });
                x += kw * extra;
            }
            for ch in row.chars() {
                let r = Rect::new(x as i32 + 2, y, (x + kw) as i32 - 2, y + kh - 4);
                cx.button(r, &ch.to_string(), Some(Hit::Key(KeyBtn::Char(ch))), key);
                x += kw;
            }
            if ri == 3 {
                let r = Rect::new(x as i32 + 2, y, (x + kw * extra) as i32 - 2, y + kh - 4);
                cx.button(r, "⌫", Some(Hit::Key(KeyBtn::Backspace)), Btn { size: 22, ..key });
            }
            y += kh;
        }
        let unit = (w - 8) as f32 / 10.0;
        let sym_lab = if self.layout == Layout::Sym { "abc" } else { "?123" };
        let items: [(&str, KeyBtn, f32); 5] = [(sym_lab, KeyBtn::Sym, 2.0), (",", KeyBtn::Char(','), 1.0),
            ("space", KeyBtn::Char(' '), 4.0), (".", KeyBtn::Char('.'), 1.0), ("⏎", KeyBtn::Enter, 2.0)];
        let mut x = 4.0;
        for (lab, k, span) in items {
            let r = Rect::new(x as i32 + 2, y, (x + unit * span) as i32 - 2, y + kh - 4);
            cx.button(r, lab, Some(Hit::Key(k)), Btn { size: 20, ..key });
            x += unit * span;
        }
    }

    /// Returns the key produced (None for modifier/layout presses).
    pub fn press(&mut self, b: KeyBtn) -> Option<Key> {
        let k = match b {
            KeyBtn::Shift => {
                self.layout = if self.layout == Layout::Lower { Layout::Upper } else { Layout::Lower };
                return None;
            }
            KeyBtn::Sym => {
                self.layout = if self.layout == Layout::Sym { Layout::Lower } else { Layout::Sym };
                return None;
            }
            KeyBtn::Ctrl => {
                self.ctrl = !self.ctrl;
                return None;
            }
            KeyBtn::Char(c) => {
                if self.layout == Layout::Upper && c.is_alphabetic() {
                    self.layout = Layout::Lower;
                }
                if self.ctrl {
                    self.ctrl = false;
                    Key::Ctrl(c.to_ascii_lowercase())
                } else {
                    Key::Char(c)
                }
            }
            KeyBtn::Backspace => Key::Backspace,
            KeyBtn::Enter => Key::Enter,
            KeyBtn::Tab => Key::Tab,
            KeyBtn::Esc => Key::Esc,
            KeyBtn::Up => Key::Up,
            KeyBtn::Down => Key::Down,
            KeyBtn::Left => Key::Left,
            KeyBtn::Right => Key::Right,
        };
        Some(k)
    }
}

/// Modal dialog: message with buttons, a menu, or a text prompt. The result goes to the
/// app that opened it via `App::dialog(tag, choice, text)`; Cancel / tapping outside is choice usize::MAX.
pub struct Dialog {
    pub tag: u32,
    pub title: String,
    pub message: String,
    pub buttons: Vec<String>,
    pub menu: Vec<String>,
    pub text: Option<String>,
    pub password: bool,
    show_pw: bool,
}

pub const CANCEL: usize = usize::MAX;

impl Dialog {
    pub fn message(title: &str, msg: &str) -> Dialog {
        Dialog { tag: 0, title: title.into(), message: msg.into(), buttons: vec!["OK".into()], menu: vec![],
                 text: None, password: false, show_pw: false }
    }
    pub fn confirm(tag: u32, title: &str, msg: &str, yes: &str) -> Dialog {
        Dialog { tag, buttons: vec!["Cancel".into(), yes.into()], ..Dialog::message(title, msg) }
    }
    pub fn menu(tag: u32, title: &str, items: Vec<String>) -> Dialog {
        Dialog { tag, menu: items, buttons: vec![], ..Dialog::message(title, "") }
    }
    pub fn input(tag: u32, title: &str, initial: &str, ok: &str) -> Dialog {
        Dialog { tag, text: Some(initial.into()), buttons: vec!["Cancel".into(), ok.into()],
                 ..Dialog::message(title, "") }
    }
    pub fn with_message(mut self, m: &str) -> Dialog {
        self.message = m.into();
        self
    }
    pub fn password(mut self) -> Dialog {
        self.password = true;
        self
    }

    /// Value passed to the app for button `i`: for confirm-style dialogs the last button is the "yes".
    fn choice_for_button(&self, i: usize) -> usize {
        if self.buttons.len() > 1 && i == 0 { CANCEL } else { i }
    }

    fn render(&mut self, cx: &mut Ctx, bottom: i32) {
        let m = 30;
        let x0 = m;
        let x1 = cx.w() - m;
        let lines = if self.message.is_empty() { vec![] } else { cx.f.wrap(&self.message, Face::Sans, 22, x1 - x0 - 40) };
        let mut h = 70 + lines.len() as i32 * 30 + if self.text.is_some() { 70 } else { 0 } + 90;
        if !self.menu.is_empty() {
            h = 70 + self.menu.len() as i32 * 66 + 20;
        }
        let top = (HEADER_H + 10).max((HEADER_H + bottom - h) / 2);
        let r = Rect::new(x0, top, x1, (top + h).min(bottom));
        cx.hits.clear();
        cx.hit(Rect::new(0, 0, cx.w(), cx.h()), Hit::Swallow, None, "");
        cx.c.fill(Rect::new(r.x0 + 6, r.y0 + 6, r.x1 + 6, r.y1 + 6), DARK);
        cx.c.fill(r, WHITE);
        cx.c.outline(r, 3, BLACK);
        cx.bold(r.x0 + 20, r.y0 + 18, &self.title.clone(), 26, BLACK, Anchor::LT);
        let mut y = r.y0 + 62;
        for ln in &lines {
            cx.text(r.x0 + 20, y, ln, 22, BLACK, Anchor::LT);
            y += 30;
        }
        if !self.menu.is_empty() {
            for (i, item) in self.menu.clone().iter().enumerate() {
                cx.button(Rect::new(r.x0 + 16, y, r.x1 - 16, y + 56), item, Some(Hit::DialogButton(i)), Btn::size(22));
                y += 66;
            }
            return;
        }
        if let Some(t) = &self.text {
            let fr = Rect::new(r.x0 + 20, y + 4, r.x1 - 20, y + 56);
            cx.c.outline(fr, 2, BLACK);
            let shown: String = if self.password && !self.show_pw { "•".repeat(t.chars().count()) } else { t.clone() };
            let tail: String = shown.chars().rev().take(38).collect::<Vec<_>>().into_iter().rev().collect();
            cx.text(fr.x0 + 10, fr.y0 + 12, &(tail + "▏"), 22, BLACK, Anchor::LT);
            if self.password {
                cx.hit(fr, Hit::DialogField, None, "field");
            }
        }
        let n = self.buttons.len() as i32;
        let bw = (r.w() - 40 - (n - 1) * 16) / n.max(1);
        let mut bx = r.x0 + 20;
        let by = r.y1 - 72;
        for (i, b) in self.buttons.clone().iter().enumerate() {
            cx.button(Rect::new(bx, by, bx + bw, by + 56), b, Some(Hit::DialogButton(i)),
                      Btn { inverted: i as i32 == n - 1, ..Btn::size(22) });
            bx += bw + 16;
        }
    }
}

pub trait App {
    fn title(&self) -> String;
    fn render(&mut self, cx: &mut Ctx, area: Rect, sys: &mut Sys);
    fn action(&mut self, _id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        Cmd::None
    }
    fn key(&mut self, _k: Key, _sys: &mut Sys) -> Cmd {
        Cmd::None
    }
    /// Some(extended) if this app shows the on-screen keyboard.
    fn keyboard(&self) -> Option<bool> {
        None
    }
    fn gesture(&mut self, _g: &Gesture, _sys: &mut Sys) -> Cmd {
        Cmd::None
    }
    fn tick(&mut self, _sys: &mut Sys) -> Cmd {
        Cmd::None
    }
    fn fd(&self) -> Option<RawFd> {
        None
    }
    fn on_fd(&mut self, _sys: &mut Sys) -> Cmd {
        Cmd::None
    }
    /// True while the app has output waiting to be shown (main loop polls faster).
    fn busy(&self) -> bool {
        false
    }
    /// Called every main-loop iteration (~100 ms when busy).
    fn poll(&mut self, _sys: &mut Sys) -> Cmd {
        Cmd::None
    }
    fn on_show(&mut self, _sys: &mut Sys) {}
    fn on_hide(&mut self, _sys: &mut Sys) {}
    fn dialog(&mut self, _tag: u32, _choice: usize, _text: String, _sys: &mut Sys) -> Cmd {
        Cmd::None
    }
    /// Apps that keep running when not on top (terminal) can be found again by key.
    fn persistent_key(&self) -> Option<&'static str> {
        None
    }
    /// Full-screen apps (the reader) get the whole screen, without the header bar.
    fn fullscreen(&self) -> bool {
        false
    }
    /// Drawing apps receive raw finger positions (below the header) instead of gestures.
    fn wants_raw_touch(&self) -> bool {
        false
    }
    fn raw_touch(&mut self, _ev: RawTouch, _sys: &mut Sys) -> Cmd {
        Cmd::None
    }
    /// Screen y where raw drawing starts; taps above it (header, toolbars) stay normal taps.
    fn raw_top(&self) -> i32 {
        HEADER_H
    }
}

#[derive(Clone, Copy, Debug)]
pub enum RawTouch {
    Down(i32, i32),
    Move(i32, i32),
    /// Lift position (x is unused by the sketchpad but part of the event).
    Up(#[allow(dead_code)] i32, i32),
}

pub struct Ui {
    pub stack: Vec<Box<dyn App>>,
    pub parked: Vec<Box<dyn App>>, // persistent apps not currently on the stack
    dialog: Option<Dialog>,
    pub kb: Keyboard,
    toast: Option<(String, Instant)>,
    cur: Canvas,
    prev: Option<Canvas>,
    /// Areas last shown with the fast black/white waveform; the next quality refresh redraws them in grey.
    fast_dirty: Option<Rect>,
    last_fast: Instant,
    pub need: Option<Update>,
    pub hits: Vec<HitBox>,
    pub quit: Option<i32>,
    pub fonts: Fonts,
}

impl Ui {
    pub fn new(w: i32, h: i32) -> Ui {
        Ui { stack: vec![], parked: vec![], dialog: None, kb: Keyboard::new(), toast: None,
             cur: Canvas::new(w, h), prev: None, fast_dirty: None, last_fast: Instant::now(), need: Some(Update::Full), hits: vec![], quit: None,
             fonts: Fonts::load() }
    }

    pub fn request(&mut self, u: Update) {
        self.need = Some(match (self.need, u) {
            (Some(Update::Full), _) | (_, Update::Full) => Update::Full,
            (Some(Update::Quality), _) | (_, Update::Quality) => Update::Quality,
            _ => Update::Fast,
        });
    }

    pub fn top(&mut self) -> &mut Box<dyn App> {
        self.stack.last_mut().unwrap()
    }

    pub fn push(&mut self, mut app: Box<dyn App>, sys: &mut Sys) {
        if let Some(t) = self.stack.last_mut() {
            t.on_hide(sys);
        }
        app.on_show(sys);
        self.stack.push(app);
        self.kb = Keyboard::new();
        self.toast = None; // notifications belong to the screen that raised them
        self.request(Update::Full);
    }

    fn park_or_drop(&mut self, app: Box<dyn App>) {
        if app.persistent_key().is_some() {
            self.parked.push(app);
        }
    }

    pub fn pop(&mut self, sys: &mut Sys) {
        if self.stack.len() > 1 {
            let mut a = self.stack.pop().unwrap();
            a.on_hide(sys);
            self.park_or_drop(a);
            self.top().on_show(sys);
            self.kb = Keyboard::new();
            self.toast = None;
            self.request(Update::Full);
        }
    }

    pub fn home(&mut self, sys: &mut Sys) {
        while self.stack.len() > 1 {
            let mut a = self.stack.pop().unwrap();
            a.on_hide(sys);
            self.park_or_drop(a);
        }
        self.top().on_show(sys);
        self.request(Update::Full);
    }

    /// Take a parked persistent app (e.g. the running terminal) if there is one.
    pub fn unpark(&mut self, key: &str) -> Option<Box<dyn App>> {
        let i = self.parked.iter().position(|a| a.persistent_key() == Some(key))?;
        Some(self.parked.remove(i))
    }

    pub fn apply(&mut self, cmd: Cmd, sys: &mut Sys) {
        match cmd {
            Cmd::None => {}
            Cmd::Redraw => self.request(Update::Quality),
            Cmd::Fast => self.request(Update::Fast),
            Cmd::Full => self.request(Update::Full),
            Cmd::Push(a) => self.push(a, sys),
            Cmd::Resume(key, make) => {
                let app = self.unpark(key).unwrap_or_else(|| make(sys));
                self.push(app, sys);
            }
            Cmd::Pop => self.pop(sys),
            Cmd::Dialog(d) => {
                self.kb = Keyboard::new();
                self.dialog = Some(d);
                self.request(Update::Quality);
            }
            Cmd::Toast(s) => {
                self.toast = Some((s, Instant::now() + Duration::from_secs(3)));
                self.request(Update::Fast);
            }
            Cmd::Quit(c) => self.quit = Some(c),
            Cmd::Batch(v) => {
                for c in v {
                    self.apply(c, sys);
                }
            }
        }
    }

    pub fn render(&mut self, disp: &mut dyn Display, sys: &mut Sys) {
        let Some(mode) = self.need.take() else { return };
        let (w, h) = (self.cur.w, self.cur.h);
        self.cur.clear(WHITE);
        let scale = if sys.config.get("ui_scale") == "large" { 1.15 } else { 1.0 };
        let mut cx = Ctx { c: &mut self.cur, f: &mut self.fonts, hits: Vec::new(), scale };
        // header
        let depth = self.stack.len();
        let app = self.stack.last_mut().unwrap();
        let full = app.fullscreen();
        if !full {
            draw_header(&mut cx, &app.title(), depth > 1, sys);
        }
        let app_kb = if self.dialog.as_ref().map_or(false, |d| d.text.is_some()) {
            None
        } else if sys.keyboard_visible {
            app.keyboard()
        } else {
            None
        };
        if let Some(ext) = app_kb {
            self.kb.extended = ext;
        }
        let bottom = h - app_kb.map_or(0, |_| self.kb.height());
        app.render(&mut cx, Rect::new(0, if full { 0 } else { HEADER_H }, w, bottom), sys);
        if app_kb.is_some() {
            self.kb.render(&mut cx, bottom);
        }
        if let Some(d) = self.dialog.as_mut() {
            if d.text.is_some() {
                self.kb.extended = false;
                let kb_top = h - self.kb.height();
                d.render(&mut cx, kb_top - 10);
                self.kb.render(&mut cx, kb_top); // keys registered last => win over the backdrop
            } else {
                d.render(&mut cx, h - 20);
            }
        }
        if let Some((msg, until)) = &self.toast {
            if Instant::now() < *until {
                let tw = cx.f.width(msg, Face::Sans, 20);
                let r = Rect::new((w - tw) / 2 - 20, HEADER_H + 12, (w + tw) / 2 + 20, HEADER_H + 56);
                cx.c.round_fill(r, 12, BLACK);
                cx.c.text(cx.f, w / 2, r.cy(), msg, Face::Sans, 20, WHITE, Anchor::MM);
            } else {
                self.toast = None;
            }
        }
        self.hits = cx.hits;
        let diff = match (&self.prev, mode) {
            (Some(p), Update::Fast | Update::Quality) => self.cur.diff_bbox(p),
            _ => Some(Rect::new(0, 0, w, h)),
        };
        let region = match mode {
            Update::Fast => {
                let Some(r) = diff else { return };
                self.fast_dirty = Some(self.fast_dirty.map_or(r, |d| d.union(&r)));
                self.last_fast = Instant::now();
                r
            }
            Update::Quality => {
                let r = match (diff, self.fast_dirty.take()) {
                    (Some(a), Some(b)) => a.union(&b),
                    (Some(a), None) | (None, Some(a)) => a,
                    (None, None) => return,
                };
                r
            }
            Update::Full => {
                self.fast_dirty = None;
                Rect::new(0, 0, w, h)
            }
        };
        disp.present(&self.cur, region, mode);
        match &mut self.prev {
            Some(p) => p.copy_from(&self.cur),
            None => {
                let mut p = Canvas::new(w, h);
                p.copy_from(&self.cur);
                self.prev = Some(p);
            }
        }
    }

    pub fn snapshot(&mut self, disp: &mut dyn Display, name: &str) {
        disp.snapshot(&self.cur, name);
    }

    pub fn find_label(&self, label: &str) -> Option<Rect> {
        self.hits.iter().rev().find(|h| h.label == label).map(|h| h.r)
            .or_else(|| self.hits.iter().rev().find(|h| h.label.starts_with(label)).map(|h| h.r))
    }

    pub fn labels(&self) -> Vec<String> {
        self.hits.iter().filter(|h| !h.label.is_empty()).map(|h| h.label.clone()).collect()
    }

    /// True while a greyscale cleanup of fast-updated areas is still pending.
    pub fn cleanup_pending(&self) -> bool {
        self.fast_dirty.is_some()
    }

    /// Two seconds after the last fast update, redraw those areas properly in greyscale.
    pub fn idle_cleanup(&mut self) {
        if self.fast_dirty.is_some() && self.need.is_none() && self.last_fast.elapsed() > Duration::from_secs(2) {
            self.request(Update::Quality);
        }
    }

    pub fn has_toast(&self) -> bool {
        self.toast.as_ref().map_or(false, |(_, until)| Instant::now() < *until)
    }

    pub fn expire_toast(&mut self) {
        if let Some((_, until)) = &self.toast {
            if Instant::now() >= *until {
                self.toast = None;
                self.request(Update::Fast);
            }
        }
    }

    pub fn gesture(&mut self, g: &Gesture, sys: &mut Sys) {
        sys.last_touch = (g.x, g.y);
        if matches!(g.kind, Kind::Tap | Kind::Long) {
            let long = g.kind == Kind::Long;
            let hit = self.hits.iter().rev().find(|h| h.r.contains(g.x, g.y)).map(|h| {
                if long { h.long.clone().unwrap_or_else(|| h.tap.clone()) } else { h.tap.clone() }
            });
            if let Some(hit) = hit {
                self.dispatch(hit, long, sys);
                return;
            }
        }
        if self.dialog.is_some() {
            return;
        }
        let cmd = self.top().gesture(g, sys);
        if let Cmd::None = cmd {
            if g.kind == Kind::SwipeRight && g.x < 60 && self.stack.len() > 1 {
                self.pop(sys);
            }
        } else {
            self.apply(cmd, sys);
        }
    }

    /// Raw finger events for drawing apps.
    pub fn raw_touch(&mut self, ev: RawTouch, sys: &mut Sys) {
        let cmd = self.top().raw_touch(ev, sys);
        self.apply(cmd, sys);
    }

    pub fn raw_top(&mut self) -> i32 {
        self.top().raw_top()
    }

    pub fn wants_raw(&mut self) -> bool {
        self.dialog.is_none() && self.top().wants_raw_touch()
    }

    fn dispatch(&mut self, hit: Hit, long: bool, sys: &mut Sys) {
        match hit {
            Hit::Back => self.pop(sys),
            Hit::Home => self.home(sys),
            Hit::Swallow => {}
            Hit::App(id) => {
                let cmd = self.top().action(id, long, sys);
                self.request(Update::Quality);
                self.apply(cmd, sys);
            }
            Hit::Key(b) => {
                let key = self.kb.press(b);
                self.request(Update::Fast);
                if let Some(k) = key {
                    if let Some(d) = self.dialog.as_mut().filter(|d| d.text.is_some()) {
                        let t = d.text.as_mut().unwrap();
                        match k {
                            Key::Backspace => {
                                t.pop();
                            }
                            Key::Enter => {
                                let last = d.buttons.len().saturating_sub(1);
                                self.close_dialog(last, sys);
                            }
                            Key::Char(c) => t.push(c),
                            _ => {}
                        }
                    } else {
                        let cmd = self.top().key(k, sys);
                        self.apply(cmd, sys);
                    }
                }
            }
            Hit::DialogField => {
                if let Some(d) = self.dialog.as_mut() {
                    d.show_pw = !d.show_pw;
                    self.request(Update::Fast);
                }
            }
            Hit::DialogButton(i) => self.close_dialog(i, sys),
        }
    }

    fn close_dialog(&mut self, i: usize, sys: &mut Sys) {
        if let Some(d) = self.dialog.take() {
            let choice = if d.menu.is_empty() { d.choice_for_button(i) } else { i };
            let text = d.text.clone().unwrap_or_default();
            self.kb = Keyboard::new();
            self.request(Update::Quality);
            if d.tag != 0 || choice != CANCEL {
                let cmd = self.top().dialog(d.tag, choice, text, sys);
                self.apply(cmd, sys);
            }
        }
    }
}

fn draw_header(cx: &mut Ctx, title: &str, nav: bool, sys: &mut Sys) {
    let w = cx.w();
    cx.hline(0, w, HEADER_H - 2, 2, BLACK);
    let mut x = 8;
    if nav {
        let b = Btn { size: 28, ..Default::default() };
        cx.button(Rect::new(6, 6, 70, HEADER_H - 8), "‹", Some(Hit::Back), b);
        cx.button(Rect::new(76, 6, 140, HEADER_H - 8), "⌂", Some(Hit::Home), Btn { size: 24, ..b });
        x = 152;
    }
    let st = sys.status();
    let mut right = Vec::new();
    if st.wifi {
        right.push("Wi-Fi".to_string());
    }
    if st.ssh {
        right.push("SSH".to_string());
    }
    if let Some(b) = st.battery {
        right.push(format!("{}{}%", if st.charging { "⚡" } else { "" }, b));
    }
    right.push(crate::sys::clock("%H:%M"));
    let rs = right.join("  ");
    let rw = cx.text(w - 10, HEADER_H / 2, &rs, 18, BLACK, Anchor::RM);
    let t = cx.ellipsize(title, Face::Bold, 24, w - x - rw - 24);
    cx.bold(x, HEADER_H / 2, &t, 24, BLACK, Anchor::LM);
}
