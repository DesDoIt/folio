//! OPDS catalogs: browse and download from Calibre's content server, other home library servers,
//! or public OPDS feeds. Catalogs (with optional basic-auth credentials) live in kos/opds.txt.

use crate::apps::open_book;
use crate::canvas::*;
use crate::epub::{decode_entities, tokenize, Tok};
use crate::getbooks::{download_book, http_get_auth, safe_filename_ext, urlencode};
use crate::input::Gesture;
use crate::sys::Sys;
use crate::ui::*;
use crate::web::url_join;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Catalog {
    pub name: String,
    pub url: String,
    pub user: String,
    pub pass: String,
}

impl Catalog {
    fn auth(&self) -> Option<(&str, &str)> {
        if self.user.is_empty() { None } else { Some((self.user.as_str(), self.pass.as_str())) }
    }
}

fn path(sys: &Sys) -> String {
    format!("{}/kos/opds.txt", sys.us)
}

pub fn load(sys: &Sys) -> Vec<Catalog> {
    std::fs::read_to_string(path(sys)).unwrap_or_default().lines().filter_map(|l| {
        let p: Vec<&str> = l.split('\t').collect();
        (p.len() >= 2).then(|| Catalog { name: p[0].into(), url: p[1].into(),
                                         user: p.get(2).unwrap_or(&"").to_string(), pass: p.get(3).unwrap_or(&"").to_string() })
    }).collect()
}

fn save(sys: &Sys, v: &[Catalog]) {
    let body: String = v.iter().map(|c| format!("{}\t{}\t{}\t{}\n", c.name, c.url, c.user, c.pass)).collect();
    let _ = std::fs::create_dir_all(format!("{}/kos", sys.us));
    let _ = std::fs::write(path(sys), body);
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub title: String,
    pub author: String,
    pub nav: Option<String>,             // sub-feed URL
    pub downloads: Vec<(String, String)>, // (mime type, URL), best first
}

#[derive(Default, Debug)]
pub struct Feed {
    pub title: String,
    pub entries: Vec<Entry>,
    pub next: Option<String>,
    pub search: Option<String>, // URL template with {searchTerms}, or an OpenSearch description URL
}

fn rank(mime: &str) -> u8 {
    match mime {
        m if m.contains("epub") => 0,
        m if m.contains("mobipocket") || m.contains("azw3") || m.contains("x-mobi") => 1,
        m if m.contains("pdf") => 2,
        m if m.contains("cbz") || m.contains("comicbook") => 3,
        _ => 9,
    }
}

pub fn ext_for(mime: &str, url: &str) -> &'static str {
    match rank(mime) {
        0 => "epub",
        1 => if url.contains("azw3") { "azw3" } else { "mobi" },
        2 => "pdf",
        3 => "cbz",
        _ => "epub",
    }
}

/// Parse an OPDS (Atom) feed; links are resolved against `base`.
pub fn parse(xml: &str, base: &str) -> Feed {
    let mut feed = Feed::default();
    let mut cur: Option<Entry> = None;
    let (mut field, mut text, mut in_author) = (None::<&str>, String::new(), false);
    for t in tokenize(xml) {
        match t {
            Tok::Open(n, a, sc) => match n.as_str() {
                "entry" => cur = Some(Entry { title: String::new(), author: String::new(), nav: None, downloads: vec![] }),
                "author" => in_author = true,
                "title" if !sc => {
                    field = Some("title");
                    text.clear();
                }
                "name" if in_author && !sc => {
                    field = Some("name");
                    text.clear();
                }
                "link" => {
                    let href = a.get("href").map(|h| url_join(base, &decode_entities(h)));
                    let rel = a.get("rel").cloned().unwrap_or_default();
                    let typ = a.get("type").cloned().unwrap_or_default();
                    let Some(href) = href else { continue };
                    match cur.as_mut() {
                        Some(e) => {
                            if rel.contains("opds-spec.org/acquisition") || (rank(&typ) < 9 && rel.is_empty()) {
                                if rank(&typ) < 9 {
                                    e.downloads.push((typ, href));
                                }
                            } else if typ.contains("atom+xml") || typ.contains("opds-catalog") || rel == "subsection" {
                                if e.nav.is_none() && !rel.contains("thumbnail") && !rel.contains("image") {
                                    e.nav = Some(href);
                                }
                            }
                        }
                        None => match rel.as_str() {
                            "next" => feed.next = Some(href),
                            "search" => {
                                if feed.search.is_none() || typ.contains("atom") {
                                    feed.search = Some(href);
                                }
                            }
                            _ => {}
                        },
                    }
                }
                _ => {}
            },
            Tok::Text(x) if field.is_some() => text.push_str(x),
            Tok::Close(n) => match n.as_str() {
                "title" | "name" => {
                    if let Some(f) = field.take() {
                        let v = decode_entities(text.trim());
                        match (cur.as_mut(), f) {
                            (Some(e), "title") => e.title = v,
                            (Some(e), "name") if e.author.is_empty() => e.author = v,
                            (None, "title") if feed.title.is_empty() => feed.title = v,
                            _ => {}
                        }
                    }
                }
                "author" => in_author = false,
                "entry" => {
                    if let Some(mut e) = cur.take() {
                        e.downloads.sort_by_key(|(m, _)| rank(m));
                        if !e.title.is_empty() && (e.nav.is_some() || !e.downloads.is_empty()) {
                            feed.entries.push(e);
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    feed
}

/// Turn a search link into a URL for `q` (handles OpenSearch description documents).
fn search_url(cat: &Catalog, link: &str, q: &str) -> Result<String, String> {
    let template = if link.contains("{searchTerms}") {
        link.to_string()
    } else {
        // OpenSearch description: find the Url template for Atom results
        let desc = http_get_auth(link, cat.auth())?;
        let mut found = None;
        for t in tokenize(&desc) {
            if let Tok::Open(n, a, _) = t {
                if n == "url" {
                    let typ = a.get("type").cloned().unwrap_or_default();
                    if let Some(tpl) = a.get("template") {
                        if found.is_none() || typ.contains("atom") {
                            found = Some(url_join(link, &decode_entities(tpl)));
                        }
                    }
                }
            }
        }
        found.ok_or("This catalog doesn't offer search")?
    };
    Ok(template.replace("{searchTerms}", &urlencode(q)).replace("{startPage?}", "1").replace("{startIndex?}", "0")
        .replace("{count?}", "30").replace("{language?}", ""))
}

enum Job {
    Feed(Result<(String, Feed), String>),
    Downloaded(Result<String, String>),
}

/// List of catalogs.
pub struct Catalogs {
    items: Vec<Catalog>,
    pager: Pager,
    sel: usize,
    draft: Vec<String>,
}

const ID_ADD: u32 = 1_000_001;
const T_URL: u32 = 1;
const T_NAME: u32 = 2;
const T_USER: u32 = 3;
const T_PASS: u32 = 4;
const T_REMOVE: u32 = 5;

impl Catalogs {
    pub fn new() -> Catalogs {
        Catalogs { items: vec![], pager: Pager::new(70), sel: 0, draft: vec![] }
    }
}

impl App for Catalogs {
    fn title(&self) -> String {
        "Catalogs (OPDS)".into()
    }

    fn on_show(&mut self, sys: &mut Sys) {
        self.items = load(sys);
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x1 - 12, a.y0 + 58), "+ Add catalog", Some(Hit::App(ID_ADD)),
                  Btn { inverted: true, ..Btn::size(20) });
        let items = &self.items;
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 70, a.x1, a.y1), items.len(),
                          "Add your Calibre server (http://<computer>:8080/opds) or any OPDS feed", |cx, r, i| {
            let c = &items[i];
            let t = cx.ellipsize(&c.name, Face::Bold, 20, r.w() - 100);
            cx.bold(r.x0 + 18, r.y0 + 12, &t, 20, BLACK, Anchor::LT);
            let u = cx.ellipsize(&c.url, Face::Sans, 14, r.w() - 100);
            cx.text(r.x0 + 18, r.y0 + 42, &u, 14, DARK, Anchor::LT);
            cx.hit(r, Hit::App(i as u32), Some(Hit::App(0x8000_0000 | i as u32)), &c.name);
            more_button(cx, r, 0x8000_0000 | i as u32);
        });
    }

    fn action(&mut self, id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        match id {
            ID_ADD => {
                self.draft.clear();
                Cmd::Dialog(Dialog::input(T_URL, "Catalog address", "http://", "Next")
                    .with_message("OPDS feed URL, e.g. http://192.168.1.10:8080/opds for Calibre"))
            }
            i if i & 0x8000_0000 != 0 => {
                self.sel = (i & 0x7fff_ffff) as usize;
                let n = self.items.get(self.sel).map(|c| c.name.clone()).unwrap_or_default();
                Cmd::Dialog(Dialog::confirm(T_REMOVE, "Remove catalog?", &n, "Remove"))
            }
            i => match self.items.get(i as usize).cloned() {
                Some(c) => Cmd::Push(Box::new(FeedView::new(c.clone(), c.url.clone(), c.name.clone()))),
                None => Cmd::None,
            },
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match tag {
            T_URL => {
                let u = text.trim().to_string();
                if !u.starts_with("http") {
                    return Cmd::Dialog(Dialog::message("Not an address", "Enter a full http(s):// address."));
                }
                let host = u.split("://").nth(1).unwrap_or(&u).split('/').next().unwrap_or(&u).to_string();
                self.draft = vec![u];
                Cmd::Dialog(Dialog::input(T_NAME, "Name", &host, "Next"))
            }
            T_NAME => {
                self.draft.push(if text.trim().is_empty() { "Catalog".into() } else { text.trim().to_string() });
                Cmd::Dialog(Dialog::input(T_USER, "Username (optional)", "", "Next")
                    .with_message("Leave empty if the catalog has no login"))
            }
            T_USER => {
                self.draft.push(text.trim().to_string());
                if text.trim().is_empty() {
                    return self.dialog(T_PASS, 1, String::new(), sys);
                }
                Cmd::Dialog(Dialog::input(T_PASS, "Password", "", "Save").password())
            }
            T_PASS => {
                let d = &self.draft;
                if d.len() >= 3 {
                    self.items.push(Catalog { url: d[0].clone(), name: d[1].clone(), user: d[2].clone(), pass: text });
                    save(sys, &self.items);
                }
                Cmd::Redraw
            }
            T_REMOVE => {
                if self.sel < self.items.len() {
                    self.items.remove(self.sel);
                    save(sys, &self.items);
                }
                Cmd::Redraw
            }
            _ => Cmd::None,
        }
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}

/// One page of a catalog feed.
pub struct FeedView {
    cat: Catalog,
    url: String,
    title: String,
    feed: Option<Feed>,
    pager: Pager,
    job: Option<Arc<Mutex<Option<Job>>>>,
    busy: Option<String>,
    sel: usize,
    last_path: Option<String>,
}

const ID_SEARCH: u32 = 1_000_001;
const ID_MORE: u32 = 1_000_002;
const T_SEARCH: u32 = 10;
const T_DOWNLOAD: u32 = 11;
const T_READ: u32 = 12;

impl FeedView {
    pub fn new(cat: Catalog, url: String, title: String) -> FeedView {
        let mut f = FeedView { cat, url: url.clone(), title, feed: None, pager: Pager::new(72), job: None, busy: None,
                               sel: 0, last_path: None };
        f.load(url);
        f
    }

    fn spawn(&mut self, label: &str, f: impl FnOnce() -> Job + Send + 'static) {
        let slot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        std::thread::spawn(move || {
            let r = f();
            *s2.lock().unwrap() = Some(r);
        });
        self.job = Some(slot);
        self.busy = Some(label.into());
    }

    fn load(&mut self, url: String) {
        let cat = self.cat.clone();
        self.spawn("Loading…", move || Job::Feed(http_get_auth(&url, cat.auth()).map(|x| {
            let f = parse(&x, &url);
            (url, f)
        })));
    }
}

impl App for FeedView {
    fn title(&self) -> String {
        self.feed.as_ref().filter(|f| !f.title.is_empty()).map_or(self.title.clone(), |f| f.title.clone())
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        let has_search = self.feed.as_ref().map_or(false, |f| f.search.is_some());
        let has_more = self.feed.as_ref().map_or(false, |f| f.next.is_some());
        let half = (a.w() - 36) / 2;
        cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x0 + 12 + half, a.y0 + 58), "Search", Some(Hit::App(ID_SEARCH)),
                  Btn { disabled: !has_search, ..Btn::size(20) });
        cx.button(Rect::new(a.x1 - 12 - half, a.y0 + 10, a.x1 - 12, a.y0 + 58), "More results ▶", Some(Hit::App(ID_MORE)),
                  Btn { disabled: !has_more, ..Btn::size(20) });
        if let Some(b) = &self.busy {
            cx.text(a.cx(), a.y0 + 80, b, 18, DARK, Anchor::MM);
        }
        let docs = format!("{}/documents", sys.us);
        let Some(feed) = &self.feed else { return };
        let entries = &feed.entries;
        self.pager.render(cx, Rect::new(a.x0, a.y0 + 100, a.x1, a.y1), entries.len(), "Empty", |cx, r, i| {
            let e = &entries[i];
            let t = cx.ellipsize(&e.title, Face::Bold, 20, r.w() - 80);
            cx.bold(r.x0 + 18, r.y0 + 12, &t, 20, BLACK, Anchor::LT);
            let sub = if e.nav.is_some() && e.downloads.is_empty() { "▸ browse".to_string() } else { e.author.clone() };
            let s = cx.ellipsize(&sub, Face::Sans, 16, r.w() - 80);
            cx.text(r.x0 + 18, r.y0 + 42, &s, 16, DARK, Anchor::LT);
            let mark = match e.downloads.first() {
                Some((m, u)) => {
                    let f = safe_filename_ext(&e.title, &e.author, ext_for(m, u));
                    if std::path::Path::new(&format!("{docs}/{f}")).exists() { "✓" } else { "↓" }
                }
                None => "▸",
            };
            cx.text(r.x1 - 22, r.cy(), mark, 24, BLACK, Anchor::RM);
            cx.hit(r, Hit::App(i as u32), None, &e.title);
        });
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn poll(&mut self, sys: &mut Sys) -> Cmd {
        let done = self.job.as_ref().and_then(|j| j.lock().unwrap().take());
        let Some(done) = done else { return Cmd::None };
        self.job = None;
        self.busy = None;
        match done {
            Job::Feed(Ok((url, f))) => {
                self.url = url;
                self.feed = Some(f);
                self.pager.page = 0;
                Cmd::Full
            }
            Job::Feed(Err(e)) => Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Couldn't load catalog", &e))]),
            Job::Downloaded(Ok(p)) => {
                self.last_path = Some(p.clone());
                let _ = sys;
                Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::confirm(T_READ, "Downloaded",
                    &format!("{} is in your library.", p.rsplit('/').next().unwrap_or("")), "Read now"))])
            }
            Job::Downloaded(Err(e)) => Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Download failed", &e))]),
        }
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        if self.job.is_some() {
            return Cmd::Toast("Still working…".into());
        }
        match id {
            ID_SEARCH => Cmd::Dialog(Dialog::input(T_SEARCH, "Search catalog", "", "Search")),
            ID_MORE => {
                if let Some(n) = self.feed.as_ref().and_then(|f| f.next.clone()) {
                    self.load(n);
                }
                Cmd::Redraw
            }
            i => {
                let Some(e) = self.feed.as_ref().and_then(|f| f.entries.get(i as usize)).cloned() else { return Cmd::None };
                self.sel = i as usize;
                if let Some((m, u)) = e.downloads.first() {
                    let dest = format!("{}/documents/{}", sys.us, safe_filename_ext(&e.title, &e.author, ext_for(m, u)));
                    if std::path::Path::new(&dest).exists() {
                        return open_book(sys, &dest);
                    }
                    return Cmd::Dialog(Dialog::confirm(T_DOWNLOAD, "Download?", &format!("{}\n{}", e.title, e.author), "Download"));
                }
                match e.nav {
                    Some(n) => Cmd::Push(Box::new(FeedView::new(self.cat.clone(), n, e.title.clone()))),
                    None => Cmd::None,
                }
            }
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match tag {
            T_SEARCH if !text.trim().is_empty() => {
                let Some(link) = self.feed.as_ref().and_then(|f| f.search.clone()) else { return Cmd::None };
                let (cat, q) = (self.cat.clone(), text.trim().to_string());
                self.spawn("Searching…", move || Job::Feed(search_url(&cat, &link, &q).and_then(|u| {
                    http_get_auth(&u, cat.auth()).map(|x| {
                        let f = parse(&x, &u);
                        (u, f)
                    })
                })));
                Cmd::Redraw
            }
            T_DOWNLOAD => {
                let Some(e) = self.feed.as_ref().and_then(|f| f.entries.get(self.sel)).cloned() else { return Cmd::None };
                let Some((m, u)) = e.downloads.first().cloned() else { return Cmd::None };
                let dest = format!("{}/documents/{}", sys.us, safe_filename_ext(&e.title, &e.author, ext_for(&m, &u)));
                let cat = self.cat.clone();
                self.spawn("Downloading…", move || Job::Downloaded(download_book(&u, &dest, cat.auth()).map(|_| dest)));
                Cmd::Redraw
            }
            T_READ => match self.last_path.clone() {
                Some(p) => open_book(sys, &p),
                None => Cmd::None,
            },
            _ => Cmd::None,
        }
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_calibre_style_feed() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>calibre Library</title>
          <link rel="search" type="application/atom+xml" href="/opds/search/{searchTerms}"/>
          <link rel="next" type="application/atom+xml;profile=opds-catalog" href="/opds/navcatalog/4e?offset=25"/>
          <entry><title>By Author</title><link type="application/atom+xml;profile=opds-catalog;kind=navigation" href="/opds/navcatalog/4f"/></entry>
          <entry><title>Pride and Prejudice</title><author><name>Jane Austen</name></author>
            <link type="application/x-mobipocket-ebook" href="/get/mobi/1" rel="http://opds-spec.org/acquisition"/>
            <link type="application/epub+zip" href="/get/epub/1" rel="http://opds-spec.org/acquisition"/>
            <link type="image/jpeg" href="/get/cover/1" rel="http://opds-spec.org/image"/></entry></feed>"#;
        let f = parse(xml, "http://192.168.1.10:8080/opds");
        assert_eq!(f.title, "calibre Library");
        assert_eq!(f.entries.len(), 2);
        assert_eq!(f.entries[0].nav.as_deref(), Some("http://192.168.1.10:8080/opds/navcatalog/4f"));
        assert_eq!(f.entries[1].author, "Jane Austen");
        assert_eq!(f.entries[1].downloads[0].1, "http://192.168.1.10:8080/get/epub/1"); // epub preferred
        assert_eq!(f.next.as_deref(), Some("http://192.168.1.10:8080/opds/navcatalog/4e?offset=25"));
        assert!(f.search.unwrap().contains("{searchTerms}"));
    }
}
