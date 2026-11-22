//! Book covers for the library's cover view: extracted from EPUB/MOBI/AZW3 files in a background
//! thread, shrunk to thumbnails and cached on disk (kos/cache/covers) so they load instantly next time.
//! Also the per-book reading progress the reader records (kos/progress.txt).

use crate::image::Gray;
use crate::sys::Sys;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const COVER_W: i32 = 150;
pub const COVER_H: i32 = 200;

pub type Covers = Arc<Mutex<HashMap<String, Option<Gray>>>>;

fn cache_file(sys_us: &str, path: &str, mtime: i64) -> String {
    let id = crate::sync::hex(&crate::sync::md5(path.as_bytes()));
    format!("{sys_us}/kos/cache/covers/{}-{mtime}.pgm", &id[..16])
}

fn read_pgm(path: &str) -> Option<Option<Gray>> {
    let b = std::fs::read(path).ok()?;
    if b.is_empty() {
        return Some(None); // known: no cover
    }
    let mut parts = b.splitn(4, |c| *c == b'\n');
    if parts.next()? != b"P5" {
        return None;
    }
    let dims = std::str::from_utf8(parts.next()?).ok()?;
    let (w, h) = dims.split_once(' ')?;
    let (w, h): (i32, i32) = (w.parse().ok()?, h.parse().ok()?);
    let _max = parts.next()?;
    let px = parts.next()?.to_vec();
    (px.len() == (w * h) as usize).then(|| Some(Gray { w, h, px }))
}

fn write_pgm(path: &str, g: Option<&Gray>) {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = match g {
        Some(g) => [format!("P5\n{} {}\n255\n", g.w, g.h).into_bytes(), g.px.clone()].concat(),
        None => vec![],
    };
    let _ = std::fs::write(path, body);
}

/// Extract and shrink a book's cover (slow: runs in the loader thread).
fn extract(path: &str) -> Option<Gray> {
    let lower = path.to_ascii_lowercase();
    if crate::pages::PAGE_EXT.iter().any(|e| lower.ends_with(&format!(".{e}"))) {
        return crate::pages::cover(path, COVER_W, COVER_H); // first page
    }
    let bytes = if lower.ends_with(".epub") {
        crate::epub::Book::open(path).ok().and_then(|mut b| b.cover_image())
    } else if lower.ends_with(".mobi") || lower.ends_with(".azw3") || lower.ends_with(".azw") {
        crate::mobi::cover(path)
    } else {
        None
    }?;
    let img = crate::image::decode(&bytes).ok()?;
    Some(crate::image::fit(&img, COVER_W, COVER_H))
}

/// Load covers for `books` ((path, mtime) pairs) into `store`, from the disk cache or the files.
pub fn load(us: &str, books: Vec<(String, i64)>, store: Covers, running: Arc<AtomicBool>) {
    let books: Vec<(String, i64)> = {
        let have = store.lock().unwrap();
        books.into_iter().filter(|(p, _)| !have.contains_key(p)).collect()
    };
    if books.is_empty() || running.swap(true, Ordering::SeqCst) {
        return; // nothing to do, or a loader is working (the page asks again when it finishes)
    }
    let us = us.to_string();
    std::thread::spawn(move || {
        for (path, mtime) in books {
            if store.lock().unwrap().contains_key(&path) {
                continue;
            }
            let cf = cache_file(&us, &path, mtime);
            let cover = match read_pgm(&cf) {
                Some(c) => c,
                None => {
                    let c = extract(&path);
                    write_pgm(&cf, c.as_ref());
                    c
                }
            };
            store.lock().unwrap().insert(path, cover);
        }
        running.store(false, Ordering::SeqCst);
    });
}

// --- reading progress per book --------------------------------------------------------------------
pub struct Progress {
    pub percent: u32,
    pub title: String,
    pub author: String,
}

fn progress_path(us: &str) -> String {
    format!("{us}/kos/progress.txt")
}

pub fn load_progress(sys: &Sys) -> HashMap<String, Progress> {
    let mut m = HashMap::new();
    for l in std::fs::read_to_string(progress_path(&sys.us)).unwrap_or_default().lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() >= 4 {
            m.insert(f[0].to_string(), Progress { percent: f[1].parse().unwrap_or(0), title: f[2].into(), author: f[3].into() });
        }
    }
    m
}

pub fn save_progress(sys: &Sys, path: &str, percent: u32, title: &str, author: &str) {
    let mut m = load_progress(sys);
    m.insert(path.to_string(), Progress { percent, title: title.into(), author: author.into() });
    let clean = |s: &str| s.replace(['\t', '\n'], " ");
    let body: String = m.iter().map(|(k, p)| format!("{k}\t{}\t{}\t{}\n", p.percent, clean(&p.title), clean(&p.author))).collect();
    let _ = std::fs::write(progress_path(&sys.us), body);
}
