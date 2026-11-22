//! Core apps: Home, Books, Files, Notes and the text Editor.

use crate::canvas::*;
use crate::input::{Gesture, Kind};
use crate::settings::{PowerApp, SettingsApp, WifiApp};
use crate::sys::{self, Sys};
use crate::getbooks::GetBooks;
use crate::reader::Reader;
use crate::term::Terminal;
use crate::web::Web;
use crate::ui::*;
use std::path::{Path, PathBuf};

const BOOK_EXT: &[&str] = &["epub", "pdf", "mobi", "azw", "azw3", "fb2", "djvu", "cbz", "cbr", "txt", "rtf", "html",
                            "htm", "doc", "docx", "chm", "kfx", "prc"];
const DRM_EXT: &[&str] = &["kfx"]; // Amazon store format; .azw/.azw3 are tried (DRM is detected when opening)
/// Formats the built-in reader handles; everything else goes to KOReader.
const READER_EXT: &[&str] = &["epub", "txt", "html", "htm", "mobi", "azw3", "azw", "prc"];
const KOREADER_EXT: &[&str] = &["epub", "pdf", "mobi", "azw3", "fb2", "djvu", "cbz", "cbr", "rtf", "doc", "docx",
                                "chm", "prc"];

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn human(n: u64) -> String {
    let mut v = n as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if v < 1024.0 || unit == "GB" {
            return if unit == "B" { format!("{v:.0} {unit}") } else { format!("{v:.1} {unit}") };
        }
        v /= 1024.0;
    }
    unreachable!()
}

/// "Three Men in a Boat_B004UJL1KK" -> "Three Men in a Boat"
pub fn book_title(fname: &str) -> String {
    let stem = Path::new(fname).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut s = stem.as_str();
    if let Some(i) = s.rfind('_') {
        let tail = &s[i + 1..];
        let asin = tail.len() == 10 && tail.starts_with("B0") && tail.chars().all(|c| c.is_ascii_alphanumeric());
        let hash = tail.len() == 32 && tail.chars().all(|c| c.is_ascii_alphanumeric());
        if asin || hash {
            s = &s[..i];
        }
    }
    s.replace("_ ", ": ").replace('_', " ").trim().to_string()
}

/// Open a book: built-in reader for EPUB/TXT, KOReader for everything else.
pub fn open_book(sys: &Sys, path: &str) -> Cmd {
    let ext = ext_of(Path::new(path));
    if READER_EXT.contains(&ext.as_str()) {
        match Reader::open(path, sys) {
            Ok(r) => return Cmd::Push(Box::new(r)),
            Err(e) => return Cmd::Dialog(Dialog::message("Can't open book", &format!("{e}\n\nTry Open in KOReader."))),
        }
    }
    open_koreader(sys, Some(path))
}

pub fn open_koreader(sys: &Sys, path: Option<&str>) -> Cmd {
    sys.request("koreader", path.unwrap_or(""));
    Cmd::Quit(10)
}

// --- Home ------------------------------------------------------------------------------
#[derive(Clone, Copy)]
enum Tile {
    Books,
    GetBooks,
    News,
    Wikipedia,
    Browser,
    Notes,
    Highlights,
    Stats,
    Games,
    Sketch,
    SendTo,
    Files,
    Terminal,
    Wifi,
    Settings,
    Power,
}

const TILES: [(&str, &str, Tile); 16] = [
    ("Books", "▤", Tile::Books),
    ("Get Books", "⇩", Tile::GetBooks),
    ("News", "☰", Tile::News),
    ("Wikipedia", "W", Tile::Wikipedia),
    ("Web", "◎", Tile::Browser),
    ("Notes", "✎", Tile::Notes),
    ("Highlights", "❝", Tile::Highlights),
    ("Stats", "▥", Tile::Stats),
    ("Games", "♞", Tile::Games),
    ("Sketch", "✐", Tile::Sketch),
    ("Send to", "⇪", Tile::SendTo),
    ("Files", "▦", Tile::Files),
    ("Terminal", ">_", Tile::Terminal),
    ("Wi-Fi", "◉", Tile::Wifi),
    ("Settings", "⚙", Tile::Settings),
    ("Power", "☾", Tile::Power),
];

pub struct Home {
    minute: String,
}

impl Home {
    pub fn new() -> Home {
        Home { minute: String::new() }
    }
}

impl App for Home {
    fn title(&self) -> String {
        "Kindle Linux".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        self.minute = sys::clock("%H:%M");
        cx.bold(a.cx(), a.y0 + 52, &self.minute.clone(), 64, BLACK, Anchor::MM);
        let wx = sys.dash.lock().unwrap().weather.clone();
        let date = sys::clock("%A, %d %B");
        let line = match wx {
            Some(w) => {
                let (desc, glyph) = crate::dashboard::describe(w.code);
                format!("{date}   ·   {glyph} {:.0}° {desc}", w.temp)
            }
            None => date,
        };
        cx.text(a.cx(), a.y0 + 104, &line, 19, DARK, Anchor::MM);
        let (cols, gap, m) = (4, 10, 14);
        let rows = (TILES.len() as i32 + cols - 1) / cols;
        let gy0 = a.y0 + 132;
        let tw = (a.w() - 2 * m - (cols - 1) * gap) / cols;
        let th = 150.min((a.y1 - gy0 - 70 - (rows - 1) * gap) / rows);
        for (i, (label, glyph, _)) in TILES.iter().enumerate() {
            let (r, c) = (i as i32 / cols, i as i32 % cols);
            let x = a.x0 + m + c * (tw + gap);
            let y = gy0 + r * (th + gap);
            let rect = Rect::new(x, y, x + tw, y + th);
            cx.c.round_rect(rect, 14, WHITE, 2, BLACK);
            cx.text(rect.cx(), rect.y0 + th / 2 - 12, glyph, 40, BLACK, Anchor::MM);
            cx.bold(rect.cx(), rect.y1 - 20, label, 16, BLACK, Anchor::MM);
            cx.hit(rect, Hit::App(i as u32), None, label);
        }
        let b = sys::battery(sys.sim);
        let (_, avail) = sys::meminfo(sys.sim);
        let ip = sys::ip_address(sys.sim).unwrap_or_else(|| "offline".into());
        let line = format!("Battery {}%   ·   RAM available {} MB   ·   {}",
                           b.capacity.map_or("?".into(), |c| c.to_string()), avail, ip);
        cx.text(a.cx(), a.y1 - 28, &line, 17, DARK, Anchor::MM);
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        let Some((_, _, tile)) = TILES.get(id as usize) else { return Cmd::None };
        match tile {
            Tile::Books => Cmd::Push(Box::new(Books::new(sys))),
            Tile::GetBooks => Cmd::Push(Box::new(GetBooks::new())),
            Tile::Browser => Cmd::Resume("web", |sys| Box::new(Web::new(sys))),
            Tile::News => Cmd::Push(Box::new(crate::news::News::new(sys))),
            Tile::Wikipedia => Cmd::Push(Box::new(crate::wiki::Wiki::new(sys))),
            Tile::Highlights => Cmd::Push(Box::new(crate::highlights::HighlightsApp::new())),
            Tile::Stats => Cmd::Push(Box::new(crate::highlights::StatsApp)),
            Tile::Games => Cmd::Push(Box::new(crate::games::GamesApp)),
            Tile::Sketch => Cmd::Push(Box::new(crate::sketch::Sketch::new())),
            Tile::SendTo => Cmd::Push(Box::new(crate::upload::SendApp::new())),
            Tile::Files => Cmd::Push(Box::new(Files::new(sys, None))),
            Tile::Terminal => Cmd::Resume("terminal", |_| Box::new(Terminal::new())),
            Tile::Notes => Cmd::Push(Box::new(Notes::new())),
            Tile::Wifi => Cmd::Push(Box::new(WifiApp::new())),
            Tile::Settings => Cmd::Push(Box::new(SettingsApp::new())),
            Tile::Power => Cmd::Push(Box::new(PowerApp)),
        }
    }

    fn tick(&mut self, _sys: &mut Sys) -> Cmd {
        if sys::clock("%H:%M") != self.minute { Cmd::Redraw } else { Cmd::None }
    }
}

// --- Books -----------------------------------------------------------------------------
struct Book {
    path: String,
    title: String,
    ext: String,
    size: u64,
    mtime: i64,
    drm: bool,
}

pub struct Books {
    pager: Pager,
    books: Vec<Book>,
    by_title: bool,
}

const ID_SORT: u32 = 1_000_000;
const ID_KOREADER: u32 = 1_000_001;

fn mtime(m: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    m.mtime()
}

impl Books {
    pub fn new(sys: &Sys) -> Books {
        let mut b = Books { pager: Pager::new(76), books: vec![], by_title: false };
        b.scan(sys);
        b
    }

    fn scan(&mut self, sys: &Sys) {
        let mut found = vec![];
        let mut stack = vec![PathBuf::from(format!("{}/documents", sys.us))];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let Ok(m) = e.metadata() else { continue };
                if m.is_dir() {
                    if !name.ends_with(".sdr") {
                        stack.push(p);
                    }
                    continue;
                }
                let ext = ext_of(&p);
                if BOOK_EXT.contains(&ext.as_str()) {
                    found.push(Book { path: p.to_string_lossy().into_owned(), title: book_title(&name),
                                      drm: DRM_EXT.contains(&ext.as_str()), ext: ext.to_uppercase(),
                                      size: m.len(), mtime: mtime(&m) });
                }
            }
        }
        if self.by_title {
            found.sort_by_key(|b| b.title.to_lowercase());
        } else {
            found.sort_by_key(|b| -b.mtime);
        }
        self.books = found;
    }
}

impl App for Books {
    fn title(&self) -> String {
        "Books".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x0 + 290, a.y0 + 62), "Open KOReader", Some(Hit::App(ID_KOREADER)),
                  Btn { inverted: true, ..Btn::size(20) });
        cx.text(a.cx(), a.y1 - 14, "Tap: read  ·  Hold: open in KOReader", 14, DARK, Anchor::MM);
        let label = if self.by_title { "Sort: A–Z" } else { "Sort: Recent" };
        cx.button(Rect::new(a.x1 - 200, a.y0 + 10, a.x1 - 12, a.y0 + 62), label, Some(Hit::App(ID_SORT)), Btn::size(20));
        let books = &self.books;
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 74, a.x1, a.y1), books.len(), "No books in documents/",
                          |cx, r, i| {
            let b = &books[i];
            let fill = if b.drm { MID } else { BLACK };
            let t = cx.ellipsize(&b.title, Face::Bold, 22, r.w() - 130);
            cx.bold(r.x0 + 20, r.y0 + 12, &t, 22, fill, Anchor::LT);
            let sub = format!("{} · {}{}", b.ext, human(b.size), if b.drm { "  ·  Amazon DRM – stock Kindle only" } else { "" });
            cx.text(r.x0 + 20, r.y0 + 44, &sub, 16, DARK, Anchor::LT);
            let tag = Rect::new(r.x1 - 92, r.y0 + 18, r.x1 - 20, r.y0 + 54);
            cx.c.round_rect(tag, 8, WHITE, 2, fill);
            let e: String = b.ext.chars().take(4).collect();
            cx.bold(tag.cx(), tag.cy(), &e, 16, fill, Anchor::MM);
            cx.hit(r, Hit::App(i as u32), Some(Hit::App(i as u32)), &b.title);
        });
    }

    fn action(&mut self, id: u32, long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        match id {
            ID_SORT => {
                self.by_title = !self.by_title;
                self.scan(sys);
                Cmd::Redraw
            }
            ID_KOREADER => open_koreader(sys, None),
            i => match self.books.get(i as usize) {
                Some(b) if b.drm => Cmd::Dialog(Dialog::message("Can't open here",
                    "This is an Amazon store book with DRM. Only the stock Kindle reader can open it. \
                     Use Power → Exit to stock Kindle.")),
                Some(b) if long => open_koreader(sys, Some(&b.path.clone())),
                Some(b) => open_book(sys, &b.path.clone()),
                None => Cmd::None,
            },
        }
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }

    fn on_show(&mut self, sys: &mut Sys) {
        self.scan(sys);
    }
}

// --- Files -----------------------------------------------------------------------------
struct Entry {
    name: String,
    path: PathBuf,
    dir: bool,
    size: u64,
    mtime: i64,
}

#[derive(Clone, Copy)]
enum FileAct {
    Read,
    Edit,
    KOReader,
    Rename,
    Delete,
}

pub struct Files {
    path: PathBuf,
    pager: Pager,
    entries: Vec<Entry>,
    hidden: bool,
    sel: usize,
    menu: Vec<FileAct>,
    error: Option<String>,
}

const ID_UP: u32 = 900_001;
const ID_NEWDIR: u32 = 900_002;
const ID_NEWFILE: u32 = 900_003;
const ID_HIDDEN: u32 = 900_004;
const TAG_MENU: u32 = 1;
const TAG_RENAME: u32 = 2;
const TAG_DELETE: u32 = 3;
const TAG_NEWDIR: u32 = 4;
const TAG_NEWFILE: u32 = 5;

impl Files {
    pub fn new(sys: &Sys, path: Option<PathBuf>) -> Files {
        let mut f = Files { path: path.unwrap_or_else(|| PathBuf::from(&sys.us)), pager: Pager::new(64), entries: vec![],
                            hidden: false, sel: 0, menu: vec![], error: None };
        f.load();
        f
    }

    fn load(&mut self) {
        self.error = None;
        let mut v = vec![];
        match std::fs::read_dir(&self.path) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') && !self.hidden {
                        continue;
                    }
                    let path = e.path();
                    let Ok(m) = std::fs::metadata(&path) else { continue };
                    v.push(Entry { name, dir: m.is_dir(), size: m.len(), mtime: mtime(&m), path });
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        v.sort_by(|a, b| b.dir.cmp(&a.dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        self.entries = v;
    }

    fn open(&mut self, i: usize, sys: &mut Sys) -> Cmd {
        let Some(e) = self.entries.get(i) else { return Cmd::None };
        if e.dir {
            self.path = e.path.clone();
            self.pager.page = 0;
            self.load();
            return Cmd::Full;
        }
        let ext = ext_of(&e.path);
        if ext == "epub" || KOREADER_EXT.contains(&ext.as_str()) {
            return open_book(sys, &e.path.to_string_lossy());
        }
        if DRM_EXT.contains(&ext.as_str()) {
            return Cmd::Toast("Amazon DRM book: stock Kindle only".into());
        }
        if ext == "png" {
            return Cmd::Push(Box::new(crate::sketch::ImageView::new(&e.path.to_string_lossy())));
        }
        if e.size <= 512 * 1024 {
            return Cmd::Push(Box::new(Editor::new(&e.path)));
        }
        self.actions(i)
    }

    fn actions(&mut self, i: usize) -> Cmd {
        let Some(e) = self.entries.get(i) else { return Cmd::None };
        self.sel = i;
        self.menu.clear();
        let mut labels = vec![];
        if !e.dir {
            if READER_EXT.contains(&ext_of(&e.path).as_str()) {
                self.menu.push(FileAct::Read);
                labels.push("Read".to_string());
            }
            self.menu.push(FileAct::Edit);
            labels.push("Edit as text".to_string());
            if KOREADER_EXT.contains(&ext_of(&e.path).as_str()) || ext_of(&e.path) == "txt" {
                self.menu.push(FileAct::KOReader);
                labels.push("Open in KOReader".into());
            }
        }
        self.menu.push(FileAct::Rename);
        labels.push("Rename".into());
        self.menu.push(FileAct::Delete);
        labels.push("Delete".into());
        Cmd::Dialog(Dialog::menu(TAG_MENU, &e.name.clone(), labels))
    }
}

impl App for Files {
    fn title(&self) -> String {
        let p = self.path.to_string_lossy();
        let chars: Vec<char> = p.chars().collect();
        if chars.len() < 28 { p.into_owned() } else { format!("…{}", chars[chars.len() - 26..].iter().collect::<String>()) }
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let b = Btn::size(20);
        let root = self.path == Path::new("/");
        cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x0 + 130, a.y0 + 58), "↑ Up", Some(Hit::App(ID_UP)),
                  Btn { disabled: root, ..b });
        cx.button(Rect::new(a.x0 + 140, a.y0 + 10, a.x0 + 290, a.y0 + 58), "+ Folder", Some(Hit::App(ID_NEWDIR)), b);
        cx.button(Rect::new(a.x0 + 300, a.y0 + 10, a.x0 + 430, a.y0 + 58), "+ File", Some(Hit::App(ID_NEWFILE)), b);
        cx.button(Rect::new(a.x1 - 150, a.y0 + 10, a.x1 - 12, a.y0 + 58), "Hidden", Some(Hit::App(ID_HIDDEN)),
                  Btn { inverted: self.hidden, ..b });
        let empty = self.error.clone().unwrap_or_else(|| "Empty folder".into());
        let entries = &self.entries;
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 68, a.x1, a.y1), entries.len(), &empty, |cx, r, i| {
            let e = &entries[i];
            cx.text(r.x0 + 20, r.cy(), if e.dir { "▸" } else { "·" }, 26, BLACK, Anchor::LM);
            let name = if e.dir { format!("{}/", e.name) } else { e.name.clone() };
            let face = if e.dir { Face::Bold } else { Face::Sans };
            let n = cx.ellipsize(&name, face, 22, r.w() - 180);
            cx.c.text(cx.f, r.x0 + 50, r.y0 + 10, &n, face, 22, BLACK, Anchor::LT);
            cx.text(r.x0 + 50, r.y0 + 38, &sys::format_time(e.mtime, "%Y-%m-%d %H:%M"), 15, DARK, Anchor::LT);
            if !e.dir {
                cx.text(r.x1 - 20, r.cy(), &human(e.size), 16, DARK, Anchor::RM);
            }
            cx.hit(r, Hit::App(i as u32), Some(Hit::App(0x8000_0000 | i as u32)), &e.name);
        });
    }

    fn action(&mut self, id: u32, long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        match id {
            ID_UP => {
                if let Some(p) = self.path.parent() {
                    self.path = p.to_path_buf();
                    self.pager.page = 0;
                    self.load();
                }
                Cmd::Full
            }
            ID_NEWDIR => Cmd::Dialog(Dialog::input(TAG_NEWDIR, "New folder", "", "Create")),
            ID_NEWFILE => Cmd::Dialog(Dialog::input(TAG_NEWFILE, "New file", "untitled.txt", "Create")),
            ID_HIDDEN => {
                self.hidden = !self.hidden;
                self.load();
                Cmd::Redraw
            }
            i if i & 0x8000_0000 != 0 => self.actions((i & 0x7fff_ffff) as usize),
            i if long => self.actions(i as usize),
            i => self.open(i as usize, sys),
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        let clean = text.trim().replace('/', "_");
        let res = match tag {
            TAG_MENU => {
                let Some(e) = self.entries.get(self.sel) else { return Cmd::None };
                match self.menu.get(choice) {
                    Some(FileAct::Read) => return open_book(sys, &e.path.to_string_lossy()),
                    Some(FileAct::Edit) => return Cmd::Push(Box::new(Editor::new(&e.path))),
                    Some(FileAct::KOReader) => return open_koreader(sys, Some(&e.path.to_string_lossy())),
                    Some(FileAct::Rename) => return Cmd::Dialog(Dialog::input(TAG_RENAME, "Rename", &e.name.clone(), "Rename")),
                    Some(FileAct::Delete) => return Cmd::Dialog(Dialog::confirm(TAG_DELETE, "Delete?",
                        &format!("Delete {}? This can't be undone.", e.name), "Delete")),
                    None => return Cmd::None,
                }
            }
            TAG_RENAME if !clean.is_empty() => {
                let e = &self.entries[self.sel];
                std::fs::rename(&e.path, self.path.join(&clean))
            }
            TAG_DELETE => {
                let e = &self.entries[self.sel];
                if e.dir { std::fs::remove_dir_all(&e.path) } else { std::fs::remove_file(&e.path) }
            }
            TAG_NEWDIR if !clean.is_empty() => std::fs::create_dir_all(self.path.join(&clean)),
            TAG_NEWFILE if !clean.is_empty() => {
                let p = self.path.join(&clean);
                let r = std::fs::OpenOptions::new().create(true).append(true).open(&p).map(|_| ());
                self.load();
                if r.is_ok() {
                    return Cmd::Push(Box::new(Editor::new(&p)));
                }
                r
            }
            _ => Ok(()),
        };
        self.load();
        match res {
            Ok(()) => Cmd::Redraw,
            Err(e) => Cmd::Toast(e.to_string()),
        }
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }

    fn on_show(&mut self, _sys: &mut Sys) {
        self.load();
    }
}

// --- Notes -----------------------------------------------------------------------------
struct Note {
    path: PathBuf,
    name: String,
    preview: String,
    mtime: i64,
}

pub struct Notes {
    pager: Pager,
    notes: Vec<Note>,
    sel: usize,
}

const ID_NEWNOTE: u32 = 1_000_000;
const TAG_DELNOTE: u32 = 1;

impl Notes {
    pub fn new() -> Notes {
        Notes { pager: Pager::new(80), notes: vec![], sel: 0 }
    }
    fn dir(sys: &Sys) -> PathBuf {
        PathBuf::from(format!("{}/notes", sys.us))
    }
}

impl App for Notes {
    fn title(&self) -> String {
        "Notes".into()
    }

    fn on_show(&mut self, sys: &mut Sys) {
        let dir = Notes::dir(sys);
        let _ = std::fs::create_dir_all(&dir);
        let mut v = vec![];
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Ok(m) = e.metadata() else { continue };
                if name.starts_with('.') || !m.is_file() {
                    continue;
                }
                let text = std::fs::read(e.path()).unwrap_or_default();
                let head = String::from_utf8_lossy(&text[..text.len().min(300)]).into_owned();
                let preview = head.lines().find(|l| !l.trim().is_empty()).unwrap_or("(empty)").trim().to_string();
                v.push(Note { path: e.path(), name, preview, mtime: mtime(&m) });
            }
        }
        v.sort_by_key(|n| -n.mtime);
        self.notes = v;
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x1 - 12, a.y0 + 62), "+ New note", Some(Hit::App(ID_NEWNOTE)),
                  Btn { inverted: true, ..Btn::size(22) });
        let notes = &self.notes;
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 74, a.x1, a.y1), notes.len(), "No notes yet", |cx, r, i| {
            let n = &notes[i];
            let p = cx.ellipsize(&n.preview, Face::Bold, 22, r.w() - 40);
            cx.bold(r.x0 + 20, r.y0 + 12, &p, 22, BLACK, Anchor::LT);
            let sub = format!("{} · {}", n.name, sys::format_time(n.mtime, "%d %b %H:%M"));
            cx.text(r.x0 + 20, r.y0 + 46, &sub, 16, DARK, Anchor::LT);
            cx.hit(r, Hit::App(i as u32), Some(Hit::App(0x8000_0000 | i as u32)), &n.preview);
        });
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        if id == ID_NEWNOTE {
            let p = Notes::dir(sys).join(format!("note-{}.txt", sys::clock("%Y%m%d-%H%M%S")));
            let _ = std::fs::write(&p, "");
            return Cmd::Push(Box::new(Editor::new(&p)));
        }
        if id & 0x8000_0000 != 0 {
            self.sel = (id & 0x7fff_ffff) as usize;
            let name = self.notes.get(self.sel).map(|n| n.name.clone()).unwrap_or_default();
            return Cmd::Dialog(Dialog::confirm(TAG_DELNOTE, "Delete note?", &name, "Delete"));
        }
        match self.notes.get(id as usize) {
            Some(n) => Cmd::Push(Box::new(Editor::new(&n.path))),
            None => Cmd::None,
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, _t: String, sys: &mut Sys) -> Cmd {
        if tag == TAG_DELNOTE && choice != CANCEL {
            if let Some(n) = self.notes.get(self.sel) {
                let _ = std::fs::remove_file(&n.path);
            }
            self.on_show(sys);
            return Cmd::Redraw;
        }
        Cmd::None
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}

// --- Editor ----------------------------------------------------------------------------
pub struct Editor {
    path: PathBuf,
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
    top: usize,
    dirty: bool,
}

const FS: u16 = 20;
const ID_SAVE: u32 = 1;
const ID_KB: u32 = 2;

impl Editor {
    pub fn new(path: &Path) -> Editor {
        let text = std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
        let row = lines.len() - 1;
        let col = lines[row].len();
        Editor { path: path.to_path_buf(), lines, row, col, top: 0, dirty: false }
    }

    fn save(&mut self) -> Cmd {
        let text: String = self.lines.iter().map(|l| l.iter().collect::<String>()).collect::<Vec<_>>().join("\n");
        match std::fs::write(&self.path, text) {
            Ok(()) => {
                self.dirty = false;
                Cmd::Toast("Saved".into())
            }
            Err(e) => Cmd::Toast(format!("Save failed: {e}")),
        }
    }

    fn edit(&mut self, k: Key) {
        let len = self.lines[self.row].len();
        match k {
            Key::Backspace => {
                if self.col > 0 {
                    self.lines[self.row].remove(self.col - 1);
                    self.col -= 1;
                } else if self.row > 0 {
                    let cur = self.lines.remove(self.row);
                    self.row -= 1;
                    self.col = self.lines[self.row].len();
                    self.lines[self.row].extend(cur);
                } else {
                    return;
                }
            }
            Key::Enter => {
                let rest = self.lines[self.row].split_off(self.col);
                self.lines.insert(self.row + 1, rest);
                self.row += 1;
                self.col = 0;
            }
            Key::Tab => {
                for _ in 0..4 {
                    self.edit(Key::Char(' '));
                }
                return;
            }
            Key::Left => {
                if self.col > 0 {
                    self.col -= 1;
                } else if self.row > 0 {
                    self.row -= 1;
                    self.col = self.lines[self.row].len();
                }
                return;
            }
            Key::Right => {
                if self.col < len {
                    self.col += 1;
                } else if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = 0;
                }
                return;
            }
            Key::Up => {
                if self.row > 0 {
                    self.row -= 1;
                    self.col = self.col.min(self.lines[self.row].len());
                }
                return;
            }
            Key::Down => {
                if self.row + 1 < self.lines.len() {
                    self.row += 1;
                    self.col = self.col.min(self.lines[self.row].len());
                }
                return;
            }
            Key::Char(c) => {
                self.lines[self.row].insert(self.col, c);
                self.col += 1;
            }
            Key::Esc | Key::Ctrl(_) => return,
        }
        self.dirty = true;
    }
}

impl App for Editor {
    fn title(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn keyboard(&self) -> Option<bool> {
        Some(true)
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let save = if self.dirty { "Save •" } else { "Save" };
        cx.button(Rect::new(a.x0 + 12, a.y0 + 8, a.x0 + 140, a.y0 + 54), save, Some(Hit::App(ID_SAVE)),
                  Btn { inverted: self.dirty, ..Btn::size(20) });
        cx.button(Rect::new(a.x0 + 150, a.y0 + 8, a.x0 + 290, a.y0 + 54), "Keyboard", Some(Hit::App(ID_KB)), Btn::size(20));
        cx.text(a.x1 - 16, a.y0 + 31, &format!("Ln {}, Col {}", self.row + 1, self.col + 1), 16, DARK, Anchor::RM);
        let cw = cx.f.mono_advance(FS);
        let lh = FS as i32 + 8;
        let (tx, ty) = (a.x0 + 14, a.y0 + 64);
        let cols = (((a.w() - 28) as f32 / cw) as usize).max(10);
        let nrows = ((a.y1 - ty - 6) / lh).max(1) as usize;
        // visual rows: (line, start col)
        let mut vis: Vec<(usize, usize)> = vec![];
        let mut cursor_vis = 0;
        for (i, l) in self.lines.iter().enumerate() {
            let chunks = (l.len().max(1) + cols - 1) / cols;
            for ci in 0..chunks {
                let start = ci * cols;
                if i == self.row && self.col >= start && (self.col < start + cols || ci == chunks - 1) {
                    cursor_vis = vis.len();
                }
                vis.push((i, start));
            }
        }
        if cursor_vis < self.top {
            self.top = cursor_vis;
        } else if cursor_vis >= self.top + nrows {
            self.top = cursor_vis + 1 - nrows;
        }
        let mut y = ty;
        for (vi, &(li, start)) in vis.iter().enumerate().skip(self.top).take(nrows) {
            let l = &self.lines[li];
            let s: String = l[start.min(l.len())..(start + cols).min(l.len())].iter().collect();
            cx.mono(tx, y, &s, FS, BLACK);
            if vi == cursor_vis {
                let x = tx + ((self.col - start) as f32 * cw) as i32;
                cx.c.fill(Rect::new(x, y - 1, x + 2, y + lh - 4), BLACK);
            }
            y += lh;
        }
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        match id {
            ID_SAVE => self.save(),
            ID_KB => {
                sys.keyboard_visible = !sys.keyboard_visible;
                Cmd::Full
            }
            _ => Cmd::None,
        }
    }

    fn key(&mut self, k: Key, _sys: &mut Sys) -> Cmd {
        if k == Key::Ctrl('s') {
            return self.save();
        }
        self.edit(k);
        Cmd::Fast
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        let k = match g.kind {
            Kind::SwipeUp => Key::Down,
            Kind::SwipeDown => Key::Up,
            _ => return Cmd::None,
        };
        for _ in 0..8 {
            self.edit(k.clone());
        }
        Cmd::Redraw
    }

    fn on_hide(&mut self, sys: &mut Sys) {
        if self.dirty {
            self.save();
        }
        sys.keyboard_visible = true;
    }
}
