//! Send to Kindle: a tiny HTTP server (port 8080) with an upload page, so books can be sent
//! from a phone or computer browser on the same Wi-Fi. Runs only while this screen is open.

use crate::canvas::*;
use crate::sys::{self, Sys};
use crate::ui::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const PORT: u16 = 8080;
const MAX_UPLOAD: usize = 80 * 1024 * 1024;
const ALLOWED: &[&str] = &["epub", "pdf", "mobi", "azw3", "txt", "html", "htm", "fb2", "djvu", "cbz", "cbr", "rtf",
                            "doc", "docx", "png", "jpg", "jpeg"];

struct Shared {
    log: Mutex<Vec<String>>,
    stop: AtomicBool,
    docs: String,
}

fn page(docs: &str, msg: &str) -> String {
    let mut books: Vec<String> = std::fs::read_dir(docs).map(|rd| rd.flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.') && !n.ends_with(".sdr") && n.contains('.'))
        .collect()).unwrap_or_default();
    books.sort_by_key(|b| b.to_lowercase());
    let list: String = books.iter().take(300).map(|b| format!("<li>{}</li>", esc(b))).collect();
    format!("<!doctype html><html><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>\
        <title>Send to Kindle</title><style>body{{font-family:system-ui,sans-serif;max-width:36em;margin:2em auto;padding:0 1em}}\
        .box{{border:2px dashed #888;padding:1.5em;border-radius:12px}}button{{font-size:1.1em;padding:.5em 1.4em}}\
        .msg{{background:#e8f5e9;padding:.8em;border-radius:8px}}li{{margin:.2em 0}}</style></head><body>\
        <h1>📖 Send to Kindle</h1>{}<div class=box><form method=post action=/upload enctype=multipart/form-data>\
        <p>Choose ebooks (EPUB, PDF, TXT, …) to put on the Kindle:</p><p><input type=file name=file multiple></p>\
        <p><button>Upload</button></p></form></div><h2>On the Kindle ({})</h2><ul>{list}</ul></body></html>",
        if msg.is_empty() { String::new() } else { format!("<p class=msg>{}</p>", esc(msg)) }, books.len())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn safe_name(n: &str) -> String {
    let base = n.rsplit(['/', '\\']).next().unwrap_or(n);
    let s: String = base.chars().map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '-' } else { c }).collect();
    s.trim().trim_start_matches('.').chars().take(150).collect()
}

fn respond(s: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                   body.len());
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Parse multipart/form-data and save file parts into `docs`. Returns saved file names.
pub fn save_multipart(body: &[u8], boundary: &str, docs: &str) -> Result<Vec<String>, String> {
    let delim = format!("--{boundary}");
    let mut saved = vec![];
    let mut pos = find(body, delim.as_bytes(), 0).ok_or("malformed upload")?;
    loop {
        pos += delim.len();
        if body.get(pos..pos + 2) == Some(b"--") {
            break; // final boundary
        }
        let hdr_end = find(body, b"\r\n\r\n", pos).ok_or("malformed part")?;
        let headers = String::from_utf8_lossy(&body[pos..hdr_end]).into_owned();
        let data_start = hdr_end + 4;
        let next = find(body, format!("\r\n{delim}").as_bytes(), data_start).ok_or("unterminated part")?;
        let data = &body[data_start..next];
        // filename="..." from the Content-Disposition line only (other header lines follow it)
        let filename = headers.lines()
            .find(|l| l.to_ascii_lowercase().starts_with("content-disposition"))
            .and_then(|l| l.split(';').find_map(|p| p.trim().strip_prefix("filename=")))
            .map(|f| f.trim().trim_matches('"').to_string());
        if let Some(f) = filename.filter(|f| !f.is_empty()) {
            let name = safe_name(&f);
            let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
            if !ALLOWED.contains(&ext.as_str()) {
                return Err(format!("{name}: .{ext} files aren't accepted"));
            }
            let path = format!("{docs}/{name}");
            std::fs::write(&path, data).map_err(|e| format!("{name}: {e}"))?;
            saved.push(name);
        }
        pos = next + 2;
    }
    Ok(saved)
}

fn handle(mut s: TcpStream, sh: &Shared) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let mut r = BufReader::new(s.try_clone().unwrap());
    let mut first = String::new();
    if r.read_line(&mut first).is_err() {
        return;
    }
    let mut len = 0usize;
    let mut ctype = String::new();
    loop {
        let mut l = String::new();
        if r.read_line(&mut l).is_err() || l == "\r\n" || l.is_empty() {
            break;
        }
        let lower = l.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        } else if lower.starts_with("content-type:") {
            ctype = l["content-type:".len()..].trim().to_string();
        }
    }
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => respond(&mut s, "200 OK", &page(&sh.docs, "")),
        ("POST", "/upload") => {
            if len == 0 || len > MAX_UPLOAD {
                respond(&mut s, "413 Too Large", &page(&sh.docs, "Upload too large (max 80 MB)."));
                return;
            }
            let mut body = vec![0u8; len];
            if r.read_exact(&mut body).is_err() {
                respond(&mut s, "400 Bad Request", &page(&sh.docs, "Upload was interrupted."));
                return;
            }
            let boundary = ctype.split(';').find_map(|p| p.trim().strip_prefix("boundary=")).unwrap_or("").trim_matches('"').to_string();
            match save_multipart(&body, &boundary, &sh.docs) {
                Ok(names) if !names.is_empty() => {
                    let mut log = sh.log.lock().unwrap();
                    for n in &names {
                        log.push(format!("{}  {n}", sys::clock("%H:%M")));
                    }
                    respond(&mut s, "200 OK", &page(&sh.docs, &format!("Sent {} file(s): {}", names.len(), names.join(", "))));
                }
                Ok(_) => respond(&mut s, "200 OK", &page(&sh.docs, "No file was chosen.")),
                Err(e) => respond(&mut s, "400 Bad Request", &page(&sh.docs, &e)),
            }
        }
        _ => respond(&mut s, "404 Not Found", "<h1>Not found</h1><a href=/>Back</a>"),
    }
}

pub struct SendApp {
    shared: Option<Arc<Shared>>,
    error: Option<String>,
    seen: usize,
}

impl SendApp {
    pub fn new() -> SendApp {
        SendApp { shared: None, error: None, seen: 0 }
    }

    fn start(&mut self, sys: &Sys) {
        let docs = format!("{}/documents", sys.us);
        let listener = match TcpListener::bind(("0.0.0.0", PORT)) {
            Ok(l) => l,
            Err(e) => {
                self.error = Some(format!("Couldn't start the server: {e}"));
                return;
            }
        };
        let _ = listener.set_nonblocking(true);
        if !sys.sim {
            // the Kindle firewall drops inbound connections; open our port while the screen is open
            let fw = sys::run(&["iptables", "-I", "INPUT", "-p", "tcp", "--dport", &PORT.to_string(), "-j", "ACCEPT"], true);
            // Wi-Fi power saving can make the Kindle miss the phone's ARP requests ("address unreachable")
            let ps = sys::run(&["iw", "dev", sys::IFACE, "set", "power_save", "off"], false);
            let rules = sys::run(&["iptables", "-S", "INPUT"], true);
            let addr = sys::run(&["ip", "-4", "-o", "addr", "show", sys::IFACE], false);
            sys::log(&format!("send-to-kindle: listening on :{PORT}; iptables: {} | power_save off: {} | {} | INPUT rules: {}",
                if fw.trim().is_empty() { "ok" } else { fw.trim() }, if ps.trim().is_empty() { "ok" } else { ps.trim() },
                addr.split_whitespace().take(4).collect::<Vec<_>>().join(" "),
                rules.lines().take(8).collect::<Vec<_>>().join(" ; ")));
        }
        let sh = Arc::new(Shared { log: Mutex::new(vec![]), stop: AtomicBool::new(false), docs });
        let s2 = sh.clone();
        std::thread::spawn(move || {
            while !s2.stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((s, _)) => {
                        let _ = s.set_nonblocking(false);
                        handle(s, &s2);
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        });
        self.shared = Some(sh);
        self.error = None;
    }

    fn stop(&mut self, sys: &Sys) {
        if let Some(sh) = self.shared.take() {
            sh.stop.store(true, Ordering::Relaxed);
            if !sys.sim {
                sys::run(&["iptables", "-D", "INPUT", "-p", "tcp", "--dport", &PORT.to_string(), "-j", "ACCEPT"], true);
                sys::run(&["iw", "dev", sys::IFACE, "set", "power_save", "on"], false);
            }
        }
    }
}

impl App for SendApp {
    fn title(&self) -> String {
        "Send to Kindle".into()
    }

    fn on_show(&mut self, sys: &mut Sys) {
        if self.shared.is_none() {
            self.start(sys);
        }
        sys.keep_awake = true;
    }

    fn on_hide(&mut self, sys: &mut Sys) {
        self.stop(sys);
        sys.keep_awake = false;
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        let mut y = a.y0 + 30;
        if let Some(e) = &self.error {
            cx.bold(a.x0 + 24, y, "Server not running", 24, BLACK, Anchor::LT);
            let lines = cx.f.wrap(e, Face::Sans, 18, a.w() - 48);
            for (i, l) in lines.iter().enumerate() {
                cx.text(a.x0 + 24, y + 44 + i as i32 * 26, l, 18, BLACK, Anchor::LT);
            }
            return;
        }
        let ip = sys::ip_address(sys.sim);
        let sz = cx.sz(18);
        for l in cx.f.wrap("On your phone or computer, connected to the same Wi-Fi, open:", Face::Sans, sz, a.w() - 48) {
            cx.text(a.x0 + 24, y, &l, 18, DARK, Anchor::LT);
            y += sz as i32 + 8;
        }
        y += 12;
        let url = match &ip {
            Some(ip) => format!("http://{ip}:{PORT}"),
            None => "Connect Wi-Fi first".into(),
        };
        let r = Rect::new(a.x0 + 20, y, a.x1 - 20, y + 84);
        cx.c.round_rect(r, 12, WHITE, 3, BLACK);
        cx.bold(r.cx(), r.cy(), &url, if ip.is_some() { 30 } else { 24 }, BLACK, Anchor::MM);
        y += 110;
        for line in ["Pick EPUB, PDF or TXT files and tap Upload. They appear in Books right away, while this screen is open.",
                     "“Address unreachable”? The phone must be on this same Wi-Fi: not on mobile data, and not a guest network (those keep devices apart)."] {
            let sz = cx.sz(17);
            for l in cx.f.wrap(line, Face::Sans, sz, a.w() - 48) {
                cx.text(a.x0 + 24, y, &l, 17, BLACK, Anchor::LT);
                y += sz as i32 + 7;
            }
            y += 8;
        }
        y += 20;
        cx.hline(a.x0 + 16, a.x1 - 16, y, 2, BLACK);
        y += 16;
        cx.bold(a.x0 + 24, y, "Received", 20, BLACK, Anchor::LT);
        y += 36;
        let log = self.shared.as_ref().map(|s| s.log.lock().unwrap().clone()).unwrap_or_default();
        if log.is_empty() {
            cx.text(a.x0 + 24, y, "Nothing yet", 18, DARK, Anchor::LT);
        }
        for l in log.iter().rev().take(10) {
            let t = cx.ellipsize(l, Face::Sans, 18, a.w() - 48);
            cx.text(a.x0 + 24, y, &t, 18, BLACK, Anchor::LT);
            y += 30;
        }
    }

    fn tick(&mut self, _sys: &mut Sys) -> Cmd {
        let n = self.shared.as_ref().map_or(0, |s| s.log.lock().unwrap().len());
        if n != self.seen {
            self.seen = n;
            return Cmd::Batch(vec![Cmd::Redraw, Cmd::Toast("Book received".into())]);
        }
        Cmd::None
    }
}

impl Drop for SendApp {
    fn drop(&mut self) {
        if let Some(sh) = &self.shared {
            sh.stop.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_saves_file() {
        let dir = std::env::temp_dir().join("kos-upload-test");
        let _ = std::fs::create_dir_all(&dir);
        let b = "----XyZ";
        let body = format!("--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"My Book.epub\"\r\n\
                            Content-Type: application/epub+zip\r\n\r\nPK-DATA\r\n--{b}--\r\n");
        let r = save_multipart(body.as_bytes(), b, dir.to_str().unwrap());
        assert_eq!(r, Ok(vec!["My Book.epub".to_string()]));
        assert_eq!(std::fs::read(dir.join("My Book.epub")).unwrap(), b"PK-DATA");
    }
}
