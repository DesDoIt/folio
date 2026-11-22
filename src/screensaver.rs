//! Sleep screens: current book (cover + progress), a random highlight, your own pictures,
//! weather, or a clean date screen. Drawn once with a full refresh, then the device suspends.

use crate::canvas::*;
use crate::dashboard;
use crate::epub::Book;
use crate::fb::{Display, Update};
use crate::image;
use crate::sys::{self, Sys};

pub const MODES: [(&str, &str); 5] = [
    ("book", "Current book"),
    ("highlight", "A highlight"),
    ("pictures", "My pictures"),
    ("weather", "Weather"),
    ("date", "Date"),
];

pub fn mode_label(key: &str) -> &'static str {
    MODES.iter().find(|m| m.0 == key).map_or("Current book", |m| m.1)
}

pub fn pictures_dir(sys: &Sys) -> String {
    format!("{}/screensavers", sys.us)
}

fn rand(n: usize) -> usize {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut x = t ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x % n.max(1) as u64) as usize
}

fn footer(c: &mut Canvas, f: &mut Fonts, battery: Option<u32>, extra: &str) {
    let (w, h) = (c.w, c.h);
    let b = battery.map_or(String::new(), |b| format!("{b}%"));
    let text = if extra.is_empty() { b } else { format!("{extra}   ·   {b}") };
    c.text(f, w / 2, h - 24, &text, Face::Sans, 14, DARK, Anchor::MM);
}

/// Draw the configured sleep screen.
pub fn draw(disp: &mut dyn Display, f: &mut Fonts, sys: &Sys, battery: Option<u32>) {
    let (w, h) = disp.size();
    let mut c = Canvas::new(w, h);
    let mode = sys.config.get("sleep_screen").to_string();
    let drawn = match mode.as_str() {
        "highlight" => highlight(&mut c, f, sys, battery),
        "pictures" => pictures(&mut c, f, sys, battery),
        "weather" => weather(disp, f, sys, battery),
        "date" => false,
        _ => book(&mut c, f, sys, battery),
    };
    if mode == "weather" && drawn {
        return; // the weather screen presents itself
    }
    if !drawn {
        let note = match mode.as_str() {
            "weather" if dashboard::configured_location(sys).is_none() => "Set a weather location in Settings",
            "weather" => "Weather unavailable (no data yet)",
            "pictures" => "Add PNG/JPG pictures to the “screensavers” folder",
            "highlight" => "No highlights yet",
            _ => "",
        };
        date(&mut c, f, battery, note);
    }
    disp.present(&c, Rect::new(0, 0, w, h), Update::Full);
}

fn date(c: &mut Canvas, f: &mut Fonts, battery: Option<u32>, note: &str) {
    let (w, h) = (c.w, c.h);
    let t = unsafe { libc::time(std::ptr::null_mut()) } as i64;
    c.text(f, w / 2, h / 2 - 210, &sys::format_time(t, "%A"), Face::Bold, 44, BLACK, Anchor::MM);
    let day = sys::format_time(t, "%d").trim_start_matches('0').to_string();
    c.text(f, w / 2, h / 2 - 40, &day, Face::Bold, 220, BLACK, Anchor::MM);
    c.text(f, w / 2, h / 2 + 120, &sys::format_time(t, "%B %Y"), Face::Sans, 32, DARK, Anchor::MM);
    c.hline(w / 2 - 60, w / 2 + 60, h / 2 + 175, 2, BLACK);
    if !note.is_empty() {
        c.text(f, w / 2, h - 70, note, Face::Sans, 16, DARK, Anchor::MM);
    }
    footer(c, f, battery, "");
}

fn book(c: &mut Canvas, f: &mut Fonts, sys: &Sys, battery: Option<u32>) -> bool {
    let line = std::fs::read_to_string(format!("{}/kos/last-book.txt", sys.us)).unwrap_or_default();
    let p: Vec<&str> = line.trim_end().split('\t').collect();
    if p.len() < 4 || !std::path::Path::new(p[0]).exists() {
        return false;
    }
    let (path, title, author) = (p[0], p[1], p[2]);
    let progress: u32 = p[3].parse().unwrap_or(0);
    let (w, h) = (c.w, c.h);
    let cover = Book::open(path).ok().and_then(|mut b| b.cover_image()).and_then(|bytes| image::decode(&bytes).ok());
    let text_top;
    match cover {
        Some(img) => {
            let g = image::fit(&img, w - 80, h - 250);
            let x = (w - g.w) / 2;
            let y = 40 + (h - 250 - g.h) / 2;
            c.blit(&g.px, g.w, g.h, x, y);
            c.outline(Rect::new(x - 1, y - 1, x + g.w + 1, y + g.h + 1), 1, DARK);
            text_top = h - 190;
        }
        None => {
            // typographic cover
            let r = Rect::new(70, 60, w - 70, h - 230);
            c.outline(r, 3, BLACK);
            c.outline(r.inset(10), 1, BLACK);
            let lines = f.wrap(title, Face::Bold, 40, r.w() - 60);
            let total = lines.len() as i32 * 52;
            let mut y = r.cy() - total / 2 - 30;
            for l in lines.iter().take(6) {
                c.text(f, w / 2, y, l, Face::Bold, 40, BLACK, Anchor::MT);
                y += 52;
            }
            if !author.is_empty() {
                c.text(f, w / 2, y + 30, author, Face::Sans, 22, DARK, Anchor::MT);
            }
            text_top = h - 190;
        }
    }
    let t = f.ellipsize(title, Face::Bold, 24, w - 60);
    c.text(f, w / 2, text_top, &t, Face::Bold, 24, BLACK, Anchor::MT);
    if !author.is_empty() {
        let a = f.ellipsize(author, Face::Sans, 18, w - 60);
        c.text(f, w / 2, text_top + 38, &a, Face::Sans, 18, DARK, Anchor::MT);
    }
    let (bx, by, bw) = (110, text_top + 82, w - 220);
    c.round_rect(Rect::new(bx, by, bx + bw, by + 14), 7, WHITE, 2, BLACK);
    let filled = (bw - 6) * progress.min(100) as i32 / 100;
    if filled > 0 {
        c.round_fill(Rect::new(bx + 3, by + 3, bx + 3 + filled, by + 11), 4, BLACK);
    }
    c.text(f, w / 2, by + 36, &format!("{progress}% read"), Face::Sans, 16, DARK, Anchor::MM);
    footer(c, f, battery, "");
    true
}

fn highlight(c: &mut Canvas, f: &mut Fonts, sys: &Sys, battery: Option<u32>) -> bool {
    let all = crate::highlights::load(sys);
    if all.is_empty() {
        return false;
    }
    let hl = &all[rand(all.len())];
    let (w, h) = (c.w, c.h);
    let box_w = w - 100;
    // largest size that fits
    let mut size = 18;
    let mut lines = vec![];
    for s in [36u16, 32, 28, 24, 21, 18] {
        let l = f.wrap(&hl.text, Face::Sans, s, box_w);
        if l.len() as i32 * (s as i32 * 150 / 100) <= h - 330 {
            size = s;
            lines = l;
            break;
        }
    }
    if lines.is_empty() {
        lines = f.wrap(&hl.text, Face::Sans, 18, box_w);
        lines.truncate(((h - 330) / 27) as usize);
    }
    let lh = size as i32 * 150 / 100;
    let total = lines.len() as i32 * lh;
    let mut y = (h - total) / 2 - 30;
    c.text(f, 50, y - 90, "“", Face::Bold, 110, LIGHT, Anchor::LT);
    for l in &lines {
        c.text(f, 50, y, l, Face::Sans, size, BLACK, Anchor::LT);
        y += lh;
    }
    c.hline(50, 110, y + 20, 2, BLACK);
    let src = f.ellipsize(&format!("— {}", hl.book), Face::Bold, 18, w - 100);
    c.text(f, 50, y + 40, &src, Face::Bold, 18, DARK, Anchor::LT);
    footer(c, f, battery, "");
    true
}

fn pictures(c: &mut Canvas, f: &mut Fonts, sys: &Sys, battery: Option<u32>) -> bool {
    let dir = pictures_dir(sys);
    let mut files: Vec<String> = std::fs::read_dir(&dir).map(|rd| rd.flatten()
        .map(|e| e.path().to_string_lossy().into_owned())
        .filter(|p| { let l = p.to_lowercase(); l.ends_with(".png") || l.ends_with(".jpg") || l.ends_with(".jpeg") })
        .collect()).unwrap_or_default();
    if files.is_empty() {
        return false;
    }
    files.sort();
    let pick = &files[rand(files.len())];
    let (w, h) = (c.w, c.h);
    // scaled copies are cached so sleeping stays quick with big photos
    let cache_dir = format!("{}/kos/cache", sys.us);
    let name = pick.rsplit('/').next().unwrap_or("x");
    let cache = format!("{cache_dir}/ss-{w}x{h}-{name}.gray");
    let src_m = std::fs::metadata(pick).and_then(|m| m.modified()).ok();
    let cache_ok = std::fs::metadata(&cache).and_then(|m| m.modified()).ok().zip(src_m).map_or(false, |(c, s)| c >= s);
    let px = if cache_ok {
        std::fs::read(&cache).ok().filter(|b| b.len() == (w * h) as usize)
    } else {
        None
    };
    let px = match px {
        Some(p) => p,
        None => {
            let Some(img) = std::fs::read(pick).ok().and_then(|b| image::decode(&b).ok()) else { return false };
            let g = image::fill(&img, w, h);
            let _ = std::fs::create_dir_all(&cache_dir);
            let _ = std::fs::write(&cache, &g.px);
            g.px
        }
    };
    c.blit(&px, w, h, 0, 0);
    // small battery badge so the picture stays clean
    if let Some(b) = battery {
        let r = Rect::new(w - 86, h - 40, w - 12, h - 12);
        c.round_rect(r, 8, WHITE, 1, DARK);
        c.text(f, r.cx(), r.cy(), &format!("{b}%"), Face::Sans, 14, BLACK, Anchor::MM);
    }
    true
}

fn weather(disp: &mut dyn Display, f: &mut Fonts, sys: &Sys, battery: Option<u32>) -> bool {
    let d = sys.dash.lock().unwrap().clone();
    if d.weather.is_none() {
        return false;
    }
    dashboard::draw_sleep(disp, f, &d, battery);
    true
}
