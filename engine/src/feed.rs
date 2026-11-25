//! News feeds: RSS 2.0 and Atom parsing (with the common extensions), finding a site's feed from
//! its address, feed dates, and turning an article into a book for the reader. No networking:
//! each platform fetches with its own HTTP stack and hands the bytes in.

use crate::epub::{decode_entities, html_to_doc, strip_css, tokenize, Block, BlockKind, Book, Run, Tok};


/// "Browse feeds": (category, name, description, address). Every address was checked to serve a feed.
pub const CATALOG: &[(&str, &str, &str, &str)] = &[
    ("India", "The Hindu", "Latest news from India and the world", "https://www.thehindu.com/feeder/default.rss"),
    ("India", "The Hindu — National", "National headlines", "https://www.thehindu.com/news/national/feeder/default.rss"),
    ("India", "The Indian Express", "News, politics, explainers", "https://indianexpress.com/feed/"),
    ("India", "Times of India", "Top stories", "https://timesofindia.indiatimes.com/rssfeedstopstories.cms"),
    ("India", "NDTV", "Top stories", "https://feeds.feedburner.com/ndtvnews-top-stories"),
    ("India", "Hindustan Times", "India news", "https://www.hindustantimes.com/feeds/rss/india-news/rssfeed.xml"),
    ("India", "India Today", "Latest news", "https://www.indiatoday.in/rss/home"),
    ("India", "BBC News — India", "India coverage from the BBC", "https://feeds.bbci.co.uk/news/world/asia/india/rss.xml"),
    ("India", "PIB", "Press Information Bureau releases", "https://pib.gov.in/RssMain.aspx?ModId=6&Lang=1&Regid=3"),
    ("Business", "Mint", "Business and economy news", "https://www.livemint.com/rss/news"),
    ("Business", "The Economic Times", "Top stories", "https://economictimes.indiatimes.com/rssfeedstopstories.cms"),
    ("Business", "Business Standard", "Top stories", "https://www.business-standard.com/rss/home_page_top_stories.rss"),
    ("Business", "BusinessLine", "From The Hindu group", "https://www.thehindubusinessline.com/feeder/default.rss"),
    ("Business", "Moneycontrol", "Markets and finance", "https://www.moneycontrol.com/rss/latestnews.xml"),
    ("Law & Opinion", "LiveLaw", "Supreme Court and High Court news", "https://www.livelaw.in/google_feeds.xml"),
    // Bar & Bench's plain feed only ever holds the newest story; the 24-hour window holds the day's
    ("Law & Opinion", "Bar & Bench", "Indian legal news, last 24 hours", "https://www.barandbench.com/stories.rss?time-period=last-24-hours"),
    ("Law & Opinion", "The Hindu — Editorials", "The Hindu's editorials", "https://www.thehindu.com/opinion/editorial/feeder/default.rss"),
    ("Sport", "ESPNcricinfo", "Cricket news", "https://www.espncricinfo.com/rss/content/story/feeds/0.xml"),
    ("Science & Tech", "The Hindu — Sci-Tech", "Science and technology", "https://www.thehindu.com/sci-tech/feeder/default.rss"),
    ("Science & Tech", "MediaNama", "Indian tech policy", "https://www.medianama.com/feed/"),
    ("Science & Tech", "Ars Technica", "Technology, science, policy", "https://feeds.arstechnica.com/arstechnica/index"),
    ("Science & Tech", "The Verge", "Technology news", "https://www.theverge.com/rss/index.xml"),
    ("Science & Tech", "Hacker News", "Front page", "https://hnrss.org/frontpage"),
    ("Science & Tech", "ScienceDaily", "Top science news", "https://www.sciencedaily.com/rss/top/science.xml"),
    ("Science & Tech", "NASA", "News releases", "https://www.nasa.gov/news-release/feed/"),
    ("World", "BBC News — World", "World news", "https://feeds.bbci.co.uk/news/world/rss.xml"),
    ("World", "The Guardian — World", "World news", "https://www.theguardian.com/world/rss"),
    ("World", "Al Jazeera", "Breaking and world news", "https://www.aljazeera.com/xml/rss/all.xml"),
    ("World", "NPR News", "US and world news", "https://www.npr.org/rss/rss.php?id=1001"),
    ("Books", "The Guardian — Books", "Reviews and book news", "https://www.theguardian.com/books/rss"),
    ("Books", "Standard Ebooks", "Newest free ebooks", "https://standardebooks.org/feeds/atom/new-releases"),
];

/// Turn what someone typed ("scroll.in", "https://example.org/feed") into an address to fetch.
pub fn normalize_address(text: &str) -> Option<String> {
    let u = text.trim();
    if u.is_empty() || u.contains(' ') || !u.contains('.') {
        return None;
    }
    Some(if u.starts_with("http://") || u.starts_with("https://") { u.to_string() } else { format!("https://{u}") })
}

#[derive(Clone, Default, Debug)]
pub struct Item {
    /// stable identity: the RSS guid or Atom id, else the link
    pub id: String,
    pub title: String,
    pub link: String,
    pub date: String,
    /// `date` as a Unix time, when it parses
    pub published: Option<i64>,
    pub content: String, // HTML (full text or summary)
    pub summary: String, // plain text teaser
    pub author: String,
    pub thumb: String, // image URL, if the feed has one
}

#[derive(Clone, Default, Debug)]
pub struct Feed {
    pub url: String,
    pub title: String,
    pub description: String,
    pub items: Vec<Item>,
}

impl Item {
    /// When it was published, as a Unix time.
    pub fn time(&self) -> Option<i64> {
        self.published.or_else(|| parse_date(&self.date))
    }

    /// Estimated reading time, only when the feed carries the full article.
    pub fn minutes(&self) -> Option<usize> {
        let words = plain_text(&self.content).split_whitespace().count();
        (words >= 300).then(|| (words + 229) / 230)
    }

    /// Whether the feed holds only a teaser, so the article page should be fetched for reading.
    pub fn needs_page(&self) -> bool {
        let len = html_to_doc(&self.content, false).0.iter().map(|b| b.chars()).sum::<usize>();
        len < 1500 && self.link.starts_with("http")
    }
}

/// An article as a one-chapter book: the fetched page in reading view when given, else the feed's
/// own text. Its links are web addresses (relative ones resolve against the article's link).
pub fn article_book(it: &Item, page: Option<&str>) -> Result<Book, String> {
    let (html, reading) = match page {
        Some(p) => (p.to_string(), true),
        None => (it.content.clone(), false),
    };
    let (mut blocks, links) = html_to_doc(&html, reading);
    if blocks.is_empty() {
        return Err("This article has no readable text.".into());
    }
    blocks.insert(0, Block { kind: BlockKind::Heading(1),
        runs: vec![Run { text: it.title.clone(), bold: true, italic: false, link: None }], start: 0 });
    let links = links.into_iter().map(|l| if l.starts_with('#') { l } else { resolve_url(&it.link, &l) }).collect();
    Ok(Book::from_blocks(&it.title, &it.author, vec![(it.title.clone(), blocks)]).with_links(links))
}

fn plain_text(html: &str) -> String {
    let (blocks, _) = html_to_doc(html, false);
    let mut out = String::new();
    for b in blocks.iter() {
        for r in b.runs.iter() {
            out.push_str(&r.text);
        }
        out.push(' ');
    }
    strip_css(&out.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn first_img(html: &str) -> Option<String> {
    tokenize(html).into_iter().find_map(|t| match t {
        Tok::Open(n, a, _) if n == "img" => a.get("src").filter(|s| s.starts_with("http") || s.starts_with("file:")).cloned(),
        _ => None,
    })
}

/// Parse RSS 2.0 or Atom, with the usual extensions (dc:creator, media:thumbnail/content, enclosures).
pub fn parse_feed(url: &str, xml: &str) -> Feed {
    let mut feed = Feed { url: url.into(), ..Default::default() };
    let mut cur: Option<Item> = None;
    let mut field: Option<String> = None;
    let mut text = String::new();
    let mut in_author = false;
    let mut in_image = false; // the channel's <image> block has its own <title>
    for t in tokenize(xml) {
        match t {
            Tok::Open(n, a, sc) => match n.as_str() {
                "item" | "entry" => cur = Some(Item::default()),
                "image" if cur.is_none() && !sc => in_image = true,
                "author" if !sc => {
                    in_author = true;
                    field = Some("author".into());
                    text.clear();
                }
                "name" if in_author => {
                    field = Some("name".into());
                    text.clear();
                }
                "thumbnail" | "content" | "enclosure" if a.contains_key("url") => {
                    // media:thumbnail / media:content / <enclosure>: keep the first image
                    let is_img = n == "thumbnail"
                        || a.get("medium").map_or(false, |m| m == "image")
                        || a.get("type").map_or(n == "content", |t| t.starts_with("image/"));
                    if let (Some(it), true) = (cur.as_mut(), is_img) {
                        if it.thumb.is_empty() {
                            it.thumb = a["url"].clone();
                        }
                    }
                }
                "title" | "pubdate" | "updated" | "published" | "date" | "description" | "summary" | "encoded"
                | "content" | "creator" | "subtitle" | "guid" | "id" => {
                    if !sc {
                        field = Some(n.clone());
                        text.clear();
                    }
                }
                "link" => {
                    let href = a.get("href").cloned();
                    let rel = a.get("rel").cloned().unwrap_or_else(|| "alternate".into());
                    match (cur.as_mut(), href) {
                        (Some(it), Some(h)) if rel == "alternate" && it.link.is_empty() => it.link = h,
                        (Some(it), Some(h)) if rel == "enclosure" && it.thumb.is_empty()
                            && a.get("type").map_or(false, |t| t.starts_with("image/")) => it.thumb = h,
                        (Some(_), None) if !sc => {
                            field = Some("link".into());
                            text.clear();
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            Tok::Text(x) if field.is_some() => text.push_str(x),
            Tok::Close(n) => {
                if n == "image" {
                    in_image = false;
                }
                if field.as_deref() == Some(n.as_str()) {
                    let v = text.trim().to_string();
                    field = None;
                    match cur.as_mut() {
                        Some(it) => match n.as_str() {
                            // the first <title> is the story's; later ones are captions (<media:title>)
                            "title" if it.title.is_empty() => it.title = decode_entities(&v),
                            "link" => it.link = v,
                            "guid" | "id" if it.id.is_empty() => it.id = decode_entities(&v),
                            "pubdate" | "updated" | "published" | "date" if it.date.is_empty() => it.date = v,
                            "encoded" | "content" => it.content = decode_entities(&v),
                            "description" | "summary" => {
                                let html = decode_entities(&v);
                                if it.summary.is_empty() {
                                    it.summary = plain_text(&html);
                                }
                                if it.content.is_empty() {
                                    it.content = html;
                                }
                            }
                            "creator" | "name" | "author" if it.author.is_empty() => {
                                // RSS <author> is often "mail@example.org (Name)"
                                let v = decode_entities(&v);
                                let v = match (v.find('('), v.rfind(')')) {
                                    (Some(a), Some(b)) if a < b && v.contains('@') => v[a + 1..b].to_string(),
                                    _ => v,
                                };
                                it.author = v.trim().to_string();
                            }
                            _ => {}
                        },
                        None if in_image => {}
                        None if n == "title" && feed.title.is_empty() => feed.title = decode_entities(&v),
                        None if (n == "description" || n == "subtitle") && feed.description.is_empty() => {
                            feed.description = plain_text(&decode_entities(&v))
                        }
                        _ => {}
                    }
                    if n == "name" {
                        field = Some("author".into()); // still inside <author>
                        text.clear();
                    }
                } else if n == "author" {
                    in_author = false;
                    field = None;
                } else if n == "item" || n == "entry" {
                    in_author = false;
                    if let Some(mut it) = cur.take() {
                        if it.summary.is_empty() {
                            it.summary = plain_text(&it.content);
                        }
                        if it.summary.chars().count() > 400 {
                            it.summary = it.summary.chars().take(400).collect();
                        }
                        if it.thumb.is_empty() {
                            it.thumb = first_img(&it.content).unwrap_or_default();
                        }
                        if it.id.is_empty() {
                            it.id = if it.link.is_empty() { it.title.clone() } else { it.link.clone() };
                        }
                        it.published = parse_date(&it.date);
                        if !it.title.is_empty() {
                            feed.items.push(it);
                        }
                    }
                }
                if n == "author" {
                    in_author = false;
                }
            }
            _ => {}
        }
    }
    if feed.title.is_empty() {
        feed.title = host(url);
    }
    feed
}

pub fn is_feed(body: &str) -> bool {
    let head: String = body.chars().take(4000).collect::<String>().to_ascii_lowercase();
    head.contains("<rss") || head.contains("<feed") || head.contains("<rdf:rdf")
}

pub fn discover_feed(base: &str, html: &str) -> Option<String> {
    let mut cands: Vec<(u8, String)> = vec![];
    for t in tokenize(html) {
        if let Tok::Open(n, a, _) = t {
            if n == "body" {
                break;
            }
            let ty = a.get("type").map(|s| s.to_ascii_lowercase()).unwrap_or_default();
            if n == "link" && a.get("rel").map_or(false, |r| r.to_ascii_lowercase().contains("alternate")) {
                let rank = if ty.contains("rss") { 0 } else if ty.contains("atom") { 1 } else { continue };
                if let Some(h) = a.get("href") {
                    cands.push((rank, h.clone()));
                }
            }
        }
    }
    cands.sort_by_key(|c| c.0);
    let href = decode_entities(&cands.first()?.1);
    Some(resolve_url(base, &href))
}

pub fn resolve_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let scheme = if base.starts_with("http://") { "http" } else { "https" };
    if let Some(rest) = href.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    let after = base.split("://").nth(1).unwrap_or(base);
    let host = after.split('/').next().unwrap_or(after);
    if href.starts_with('/') {
        return format!("{scheme}://{host}{href}");
    }
    let dir = base.rsplit_once('/').map_or(base, |(d, _)| d);
    format!("{dir}/{href}")
}

pub fn host(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    if url.starts_with("file:") {
        return rest.rsplit('/').next().unwrap_or(rest).to_string();
    }
    rest.split('/').next().unwrap_or(rest).trim_start_matches("www.").to_string()
}

// --- Dates -----------------------------------------------------------------------------------------
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// RFC 822 ("Sat, 21 Nov 2026 09:00:00 +0100") or ISO 8601 ("2026-11-21T09:00:00Z") to a Unix time.
pub fn parse_date(d: &str) -> Option<i64> {
    let d = d.trim();
    let num = |s: &str| s.parse::<i64>().ok();
    let tz = |z: &str| -> i64 {
        let z = z.trim();
        if let Some(sign) = z.chars().next().filter(|c| *c == '+' || *c == '-') {
            let digits: String = z[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 4 {
                let off = num(&digits[..2]).unwrap_or(0) * 3600 + num(&digits[2..4]).unwrap_or(0) * 60;
                return if sign == '+' { -off } else { off };
            }
        }
        match z {
            "EST" => 5 * 3600, "EDT" => 4 * 3600, "CST" => 6 * 3600, "CDT" => 5 * 3600,
            "MST" => 7 * 3600, "MDT" => 6 * 3600, "PST" => 8 * 3600, "PDT" => 7 * 3600,
            _ => 0,
        }
    };
    let hms = |t: &str| -> Option<i64> {
        let mut p = t.split(':');
        Some(num(p.next()?)? * 3600 + num(p.next()?)? * 60 + p.next().and_then(|s| num(s.get(..2).unwrap_or(s))).unwrap_or(0))
    };
    if d.len() >= 10 && d.as_bytes().get(4) == Some(&b'-') {
        let day = days_from_civil(num(&d[0..4])?, num(&d[5..7])?, num(&d[8..10])?);
        let rest = d.get(11..).unwrap_or("");
        let (time, zone) = match rest.find(|c| c == 'Z' || c == '+' || c == '-') {
            Some(p) => (&rest[..p], &rest[p..]),
            None => (rest, ""),
        };
        let secs = if time.is_empty() { 0 } else { hms(time.split('.').next().unwrap_or(time))? };
        return Some(day * 86400 + secs + if zone == "Z" { 0 } else { tz(zone) });
    }
    let parts: Vec<&str> = d.split_whitespace().filter(|p| !p.ends_with(',')).collect();
    let (dd, mon, yy, time) = (num(parts.first()?)?, parts.get(1)?, num(parts.get(2)?)?, parts.get(3)?);
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let m = MONTHS.iter().position(|x| mon.to_ascii_lowercase().starts_with(x))? as i64 + 1;
    let yy = if yy < 100 { yy + 2000 } else { yy };
    Some(days_from_civil(yy, m, dd) * 86400 + hms(time)? + tz(parts.get(4).unwrap_or(&"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_with_guid_and_date() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Example News</title>
            <item><title>First &amp; best</title><link>https://example.org/a</link><guid isPermaLink="false">tag:1</guid>
            <pubDate>Sat, 21 Nov 2026 09:00:00 +0100</pubDate><description>&lt;p&gt;Hello &lt;b&gt;world&lt;/b&gt;&lt;/p&gt;</description></item>
            <item><title>Second</title><link>https://example.org/b</link></item></channel></rss>"#;
        let f = parse_feed("https://example.org/feed", xml);
        assert_eq!(f.title, "Example News");
        assert_eq!(f.items.len(), 2);
        assert_eq!(f.items[0].title, "First & best");
        assert_eq!(f.items[0].id, "tag:1");
        assert_eq!(f.items[0].published, parse_date("2026-11-21T08:00:00Z"), "09:00 +0100 is 08:00 UTC");
        assert_eq!(f.items[0].summary, "Hello world");
        assert_eq!(f.items[1].id, "https://example.org/b", "no guid: the link identifies it");
    }

    #[test]
    fn parses_atom_ids() {
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Blog</title><id>urn:feed</id>
            <entry><title>Post</title><id>urn:post:1</id><link rel="alternate" href="https://b.example/p1"/>
            <updated>2026-11-21T08:00:00Z</updated><author><name>Asha</name></author><summary>Hi</summary></entry></feed>"#;
        let f = parse_feed("https://b.example/atom", xml);
        assert_eq!(f.title, "Blog");
        assert_eq!(f.items[0].id, "urn:post:1");
        assert_eq!(f.items[0].link, "https://b.example/p1");
        assert_eq!(f.items[0].author, "Asha");
        assert_eq!(f.items[0].published, parse_date("2026-11-21T08:00:00Z"));
        assert!(f.items[0].published.is_some());
    }

    #[test]
    fn finds_a_sites_feed() {
        let html = r#"<html><head><link rel="alternate" type="application/atom+xml" href="/atom.xml">
            <link rel="alternate" type="application/rss+xml" href="/feed/"></head><body></body></html>"#;
        assert_eq!(discover_feed("https://news.example/today", html).as_deref(), Some("https://news.example/feed/"));
        assert_eq!(normalize_address("scroll.in").as_deref(), Some("https://scroll.in"));
    }

    #[test]
    fn article_becomes_a_book_with_absolute_links() {
        let it = Item { title: "T".into(), link: "https://site.example/news/a".into(),
                        content: r#"<p>Read <a href="/more">more</a>.</p>"#.into(), ..Default::default() };
        let mut b = article_book(&it, None).unwrap();
        let blocks = b.chapter_blocks(0);
        assert!(matches!(blocks[0].kind, BlockKind::Heading(1)));
        assert_eq!(b.chapter_doc(0).links, vec!["https://site.example/more".to_string()]);
    }
}
