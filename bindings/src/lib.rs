//! The engine as seen from Kotlin and Swift. Only translation lives here: plain records in and
//! out, one `ReadingSession` object per open book. All behaviour is in the `engine` crate.

use engine::canvas::Fonts;
use engine::layout::Layout;
use engine::session::{self, Session, Tap, Theme};
use std::sync::{Arc, Mutex};

uniffi::setup_scaffolding!();

#[derive(Debug, uniffi::Error)]
pub enum EngineError {
    /// the book could not be opened or read
    Failed { reason: String },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Failed { reason } => f.write_str(reason),
        }
    }
}

impl std::error::Error for EngineError {}

fn failed(reason: String) -> EngineError {
    EngineError::Failed { reason }
}

#[derive(uniffi::Record, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub chapter: u64,
    pub offset: u64,
}

impl From<session::Position> for Position {
    fn from(p: session::Position) -> Self {
        Position { chapter: p.chapter as u64, offset: p.offset as u64 }
    }
}

impl From<Position> for session::Position {
    fn from(p: Position) -> Self {
        session::Position { chapter: p.chapter as usize, offset: p.offset as usize }
    }
}

/// Page look, in pixels of the screen the pages are drawn for.
#[derive(uniffi::Record, Clone, Copy, Debug)]
pub struct PageStyle {
    pub font_size: u16,
    /// left and right margin
    pub margin: i32,
    /// space above the first line and below the last
    pub top: i32,
    pub bottom: i32,
    /// line height as a percentage of the font size (e.g. 145)
    pub line_spacing: i32,
    pub hyphenate: bool,
}

/// Page colours as 0xRRGGBB.
#[derive(uniffi::Record, Clone, Copy, Debug)]
pub struct PageTheme {
    pub paper: u32,
    pub ink: u32,
    /// draw pictures in their own colours rather than recoloured with the theme
    pub natural_images: bool,
}

/// The fonts to draw with, supplied by the app (the engine bundles none): TrueType/OpenType data
/// for the four faces, and font files to search, in order, for scripts those faces lack.
#[derive(uniffi::Record, Clone, Debug)]
pub struct FontFiles {
    pub regular: Vec<u8>,
    pub bold: Vec<u8>,
    pub italic: Vec<u8>,
    pub mono: Vec<u8>,
    pub fallbacks: Vec<String>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TocItem {
    pub title: String,
    pub position: Position,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct SearchResult {
    pub snippet: String,
    pub chapter_title: String,
    pub position: Position,
}

#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum TapResult {
    Nothing,
    Jump { position: Position },
    Note { text: String, target: Position },
    External { url: String },
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct WordHit {
    pub text: String,
    pub position: Position,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct BookMeta {
    pub title: String,
    pub author: String,
    /// cover image bytes as stored in the book (JPEG/PNG/GIF)
    pub cover: Option<Vec<u8>>,
}

fn rgb(c: u32) -> [u8; 3] {
    [(c >> 16) as u8, (c >> 8) as u8, c as u8]
}

fn load_fonts(f: &FontFiles) -> Result<Fonts, EngineError> {
    Fonts::new(&f.regular, &f.bold, &f.italic, &f.mono, f.fallbacks.clone()).map_err(failed)
}

/// Title, author and cover of a book file, for a library screen.
#[uniffi::export]
pub fn book_meta(path: String) -> Result<BookMeta, EngineError> {
    let i = engine::book_info(&path).map_err(failed)?;
    Ok(BookMeta { title: i.title, author: i.author, cover: i.cover })
}

// --- news feeds ---

/// One article from a feed.
#[derive(uniffi::Record, Clone, Debug)]
pub struct FeedItem {
    /// stable identity (RSS guid / Atom id / link): use it to tell new articles from seen ones
    pub id: String,
    pub title: String,
    pub link: String,
    /// Unix time, when the feed gives a date that parses
    pub published: Option<i64>,
    /// plain-text teaser
    pub summary: String,
    /// HTML: the full article or a summary
    pub content: String,
    pub author: String,
    /// image URL, if any
    pub thumb: String,
    /// estimated reading minutes, when the feed carries the full text
    pub minutes: Option<u32>,
    /// the feed holds only a teaser: fetch `link` and pass the page to `open_article`
    pub needs_page: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct ParsedFeed {
    pub url: String,
    pub title: String,
    pub description: String,
    pub items: Vec<FeedItem>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct CatalogFeed {
    pub category: String,
    pub name: String,
    pub description: String,
    pub url: String,
}

impl From<&engine::feed::Item> for FeedItem {
    fn from(i: &engine::feed::Item) -> Self {
        FeedItem { id: i.id.clone(), title: i.title.clone(), link: i.link.clone(), published: i.published,
                   summary: i.summary.clone(), content: i.content.clone(), author: i.author.clone(),
                   thumb: i.thumb.clone(), minutes: i.minutes().map(|m| m as u32), needs_page: i.needs_page() }
    }
}

impl From<&FeedItem> for engine::feed::Item {
    fn from(i: &FeedItem) -> Self {
        engine::feed::Item { id: i.id.clone(), title: i.title.clone(), link: i.link.clone(), date: String::new(),
                             published: i.published, content: i.content.clone(), summary: i.summary.clone(),
                             author: i.author.clone(), thumb: i.thumb.clone() }
    }
}

/// Parse an RSS or Atom document fetched from `url`.
#[uniffi::export]
pub fn parse_feed(url: String, body: String) -> ParsedFeed {
    let f = engine::feed::parse_feed(&url, &body);
    ParsedFeed { url: f.url, title: f.title, description: f.description, items: f.items.iter().map(FeedItem::from).collect() }
}

/// Whether a fetched document is a feed (rather than a web page).
#[uniffi::export]
pub fn looks_like_feed(body: String) -> bool {
    engine::feed::is_feed(&body)
}

/// The feed a web page advertises, as an absolute address.
#[uniffi::export]
pub fn find_feed_link(page_url: String, html: String) -> Option<String> {
    engine::feed::discover_feed(&page_url, &html)
}

/// What someone typed ("scroll.in") as an address to fetch, or none if it can't be one.
#[uniffi::export]
pub fn feed_address(text: String) -> Option<String> {
    engine::feed::normalize_address(&text)
}

/// Suggested feeds, by category.
#[uniffi::export]
pub fn feed_catalog() -> Vec<CatalogFeed> {
    engine::feed::CATALOG.iter().map(|(c, n, d, u)| CatalogFeed { category: c.to_string(), name: n.to_string(),
                                                                   description: d.to_string(), url: u.to_string() })
        .collect()
}

// --- getting books ---

/// A book that can be downloaded.
#[derive(uniffi::Record, Clone, Debug)]
pub struct FoundBook {
    pub title: String,
    pub author: String,
    /// source, size or popularity
    pub info: String,
    /// download addresses for the same book, tried in order
    pub urls: Vec<String>,
    /// extension of the downloaded file ("epub", "azw3", ...)
    pub ext: String,
    /// cover image address, or empty
    pub cover: String,
}

impl From<engine::books::Found> for FoundBook {
    fn from(f: engine::books::Found) -> Self {
        FoundBook { title: f.title, author: f.author, info: f.info, urls: f.urls, ext: f.ext, cover: f.cover }
    }
}

/// A page of results and the address of the next page, if there is one.
#[derive(uniffi::Record, Clone, Debug)]
pub struct FoundPage {
    pub books: Vec<FoundBook>,
    pub next: Option<String>,
}

/// One entry of an OPDS catalog: a sub-catalog to open, or a book.
#[derive(uniffi::Record, Clone, Debug)]
pub struct CatalogEntry {
    pub title: String,
    pub author: String,
    /// sub-catalog address (navigation entries)
    pub nav: Option<String>,
    /// the book, when the entry can be downloaded
    pub book: Option<FoundBook>,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct CatalogPage {
    pub title: String,
    pub entries: Vec<CatalogEntry>,
    pub next: Option<String>,
    /// a search template with {searchTerms}, or an OpenSearch description to fetch first
    pub search: Option<String>,
}

/// Project Gutenberg: popular books (empty query) or a search, as an address to fetch.
#[uniffi::export]
pub fn gutenberg_address(query: String) -> String {
    engine::books::gutenberg_url(Some(&query))
}

/// A fetched Project Gutenberg (Gutendex) page.
#[uniffi::export]
pub fn parse_gutenberg(body: String) -> Result<FoundPage, EngineError> {
    let (books, next) = engine::books::parse_gutendex(&body).map_err(failed)?;
    Ok(FoundPage { books: books.into_iter().map(Into::into).collect(), next })
}

/// Standard Ebooks: newest releases (empty query) or a search, as an address to fetch.
#[uniffi::export]
pub fn standard_ebooks_address(query: String) -> String {
    engine::books::standard_ebooks_url(Some(&query))
}

/// A fetched Standard Ebooks feed.
#[uniffi::export]
pub fn parse_standard_ebooks(body: String) -> Result<FoundPage, EngineError> {
    let books = engine::books::parse_standard_ebooks(&body).map_err(failed)?;
    Ok(FoundPage { books: books.into_iter().map(Into::into).collect(), next: None })
}

/// A fetched OPDS catalog page; `name` labels its books.
#[uniffi::export]
pub fn parse_catalog(body: String, url: String, name: String) -> CatalogPage {
    let f = engine::books::parse_opds(&body, &url);
    CatalogPage {
        title: f.title,
        entries: f.entries.iter().map(|e| CatalogEntry { title: e.title.clone(), author: e.author.clone(), nav: e.nav.clone(),
                                                          book: e.found(&name).map(Into::into) }).collect(),
        next: f.next,
        search: f.search,
    }
}

/// The search address for `query`: from a template directly, or from a fetched OpenSearch
/// description (`description` = its body, fetched from `link`).
#[uniffi::export]
pub fn catalog_search_address(link: String, description: Option<String>, query: String) -> Option<String> {
    let template = if link.contains("{searchTerms}") {
        link
    } else {
        engine::books::opensearch_template(&description?, &link)?
    };
    Some(engine::books::fill_search(&template, &query))
}

/// One book in a merged result list, with the index of the list it came from.
#[derive(uniffi::Record, Clone, Debug)]
pub struct MergedBook {
    pub source: u32,
    pub book: FoundBook,
}

/// Mix results from several sources: alternating, in the order given (best editions first), with
/// later copies of the same book dropped.
#[uniffi::export]
pub fn merge_results(lists: Vec<Vec<FoundBook>>, query: String) -> Vec<MergedBook> {
    let lists: Vec<Vec<engine::books::Found>> = lists.into_iter().map(|l| l.into_iter().map(|f| engine::books::Found {
        title: f.title, author: f.author, info: f.info, urls: f.urls, ext: f.ext, cover: f.cover }).collect()).collect();
    engine::books::merge_results(&lists, &query).into_iter().map(|(s, f)| MergedBook { source: s as u32, book: f.into() }).collect()
}

fn found_list(v: Vec<engine::books::Found>) -> Vec<FoundBook> {
    v.into_iter().map(Into::into).collect()
}

fn to_found(b: FoundBook) -> engine::books::Found {
    engine::books::Found { title: b.title, author: b.author, info: b.info, urls: b.urls, ext: b.ext, cover: b.cover }
}

/// A Wikisource language edition.
#[derive(uniffi::Record, Clone, Debug)]
pub struct WikisourceLanguage {
    pub code: String,
    pub name: String,
}

#[uniffi::export]
pub fn wikisource_languages() -> Vec<WikisourceLanguage> {
    engine::books::WIKISOURCE_LANGUAGES.iter().map(|(c, n)| WikisourceLanguage { code: c.to_string(), name: n.to_string() }).collect()
}

/// Wikisource search in one language edition, as an address to fetch.
#[uniffi::export]
pub fn wikisource_address(lang: String, query: String) -> String {
    engine::books::wikisource_search_url(&lang, &query)
}

#[uniffi::export]
pub fn parse_wikisource(lang: String, body: String) -> Result<Vec<FoundBook>, EngineError> {
    engine::books::parse_wikisource(&lang, &body).map(found_list).map_err(failed)
}

/// The whole Project Gutenberg Australia catalogue (one text file).
#[uniffi::export]
pub fn gutenberg_australia_catalog_address() -> String {
    engine::books::PGA_INDEX_URL.to_string()
}

#[uniffi::export]
pub fn parse_gutenberg_australia_catalog(body: String) -> Vec<FoundBook> {
    found_list(engine::books::parse_pga_index(&body))
}

/// One page of Faded Page's complete list, starting at `start` (250 books a page).
#[uniffi::export]
pub fn faded_page_list_address(start: u32) -> String {
    engine::books::fadedpage_list_url(start as usize)
}

/// A page of Faded Page's list: its books and the size of the whole catalogue.
#[derive(uniffi::Record, Clone, Debug)]
pub struct FadedPageList {
    pub books: Vec<FoundBook>,
    pub total: u32,
}

#[uniffi::export]
pub fn parse_faded_page_list(body: String) -> FadedPageList {
    let (books, total) = engine::books::parse_fadedpage_list(&body);
    FadedPageList { books: found_list(books), total: total as u32 }
}

/// Internet Archive search, limited to public-domain texts with an EPUB.
#[uniffi::export]
pub fn internet_archive_address(query: String) -> String {
    engine::books::archive_search_url(&query)
}

#[uniffi::export]
pub fn parse_internet_archive(body: String) -> Result<Vec<FoundBook>, EngineError> {
    engine::books::parse_archive(&body).map(found_list).map_err(failed)
}

/// Search a catalogue kept on the device (title and author; every word must match).
#[uniffi::export]
pub fn search_catalog(books: Vec<FoundBook>, query: String, limit: u32) -> Vec<FoundBook> {
    let v: Vec<engine::books::Found> = books.into_iter().map(to_found).collect();
    found_list(engine::books::search_catalog(&v, &query, limit as usize))
}

/// A safe file name for a downloaded book.
#[uniffi::export]
pub fn book_file_name(title: String, author: String, ext: String) -> String {
    engine::books::file_name(&title, &author, &ext)
}

/// One open book laid out for one screen. Thread-safe; calls are serialised.
#[derive(uniffi::Object)]
pub struct ReadingSession {
    inner: Mutex<Session>,
}

impl ReadingSession {
    fn s(&self) -> std::sync::MutexGuard<'_, Session> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[uniffi::export]
impl ReadingSession {
    /// Open a book: EPUB, MOBI/AZW3/AZW/PRC, FB2/FB2.ZIP, RTF, DOCX, HTML or TXT.
    #[uniffi::constructor]
    pub fn open(path: String, fonts: FontFiles) -> Result<Arc<Self>, EngineError> {
        let s = Session::open(&path, load_fonts(&fonts)?).map_err(failed)?;
        Ok(Arc::new(ReadingSession { inner: Mutex::new(s) }))
    }

    /// Open a news article for reading: the fetched page (reading view) if given, else the
    /// feed's own text.
    #[uniffi::constructor]
    pub fn open_article(item: FeedItem, page: Option<String>, fonts: FontFiles) -> Result<Arc<Self>, EngineError> {
        let book = engine::feed::article_book(&(&item).into(), page.as_deref()).map_err(failed)?;
        Ok(Arc::new(ReadingSession { inner: Mutex::new(Session::with_book(book, load_fonts(&fonts)?)) }))
    }

    pub fn title(&self) -> String {
        self.s().title().to_string()
    }

    pub fn author(&self) -> String {
        self.s().author().to_string()
    }

    pub fn toc(&self) -> Vec<TocItem> {
        self.s().toc().into_iter().map(|t| TocItem { title: t.title, position: t.position.into() }).collect()
    }

    pub fn chapter_title(&self) -> String {
        self.s().chapter_title()
    }

    pub fn set_viewport(&self, width: i32, height: i32) {
        self.s().set_viewport(width, height)
    }

    pub fn set_style(&self, style: PageStyle) {
        let layout = Layout { margin: style.margin, top: style.top, bottom: style.bottom, spacing: style.line_spacing,
                              hyphenate: style.hyphenate };
        self.s().set_style(session::Style { font_size: style.font_size.max(6), layout })
    }

    pub fn set_fonts(&self, fonts: FontFiles) -> Result<(), EngineError> {
        self.s().set_fonts(load_fonts(&fonts)?);
        Ok(())
    }

    pub fn position(&self) -> Position {
        self.s().position().into()
    }

    pub fn goto(&self, position: Position) {
        self.s().goto(position.into())
    }

    /// Turn `delta` pages forward (positive) or back; false at either end of the book.
    pub fn turn(&self, delta: i32) -> bool {
        self.s().turn(delta)
    }

    /// The page `delta` pages from the current one as RGBA bytes (width × height × 4), or none
    /// past either end of the book.
    pub fn render(&self, delta: i32, theme: PageTheme) -> Option<Vec<u8>> {
        let t = Theme { paper: rgb(theme.paper), ink: rgb(theme.ink), natural_images: theme.natural_images };
        self.s().render_rgba(delta, t)
    }

    /// How far through the book the current page is, 0.0-1.0.
    pub fn progress(&self) -> f32 {
        self.s().progress()
    }

    pub fn goto_progress(&self, fraction: f32) {
        self.s().goto_progress(fraction)
    }

    /// Page number within the current chapter (from 0) and the chapter's page count.
    pub fn page_in_chapter(&self) -> Vec<u64> {
        let (p, n) = self.s().page_in_chapter();
        vec![p as u64, n as u64]
    }

    pub fn tap(&self, x: i32, y: i32) -> TapResult {
        match self.s().tap(x, y) {
            Tap::None => TapResult::Nothing,
            Tap::Jump(p) => TapResult::Jump { position: p.into() },
            Tap::Note { text, target } => TapResult::Note { text, target: target.into() },
            Tap::External(url) => TapResult::External { url },
        }
    }

    pub fn word_at(&self, x: i32, y: i32) -> Option<WordHit> {
        self.s().word_at(x, y).map(|w| WordHit { text: w.text, position: w.position.into(), left: w.rect.x0,
                                                 top: w.rect.y0, right: w.rect.x1, bottom: w.rect.y1 })
    }

    pub fn page_snippet(&self, words: u32) -> String {
        self.s().page_snippet(words as usize)
    }

    pub fn on_current_page(&self, position: Position) -> bool {
        self.s().on_current_page(position.into())
    }

    pub fn search(&self, query: String, limit: u32) -> Vec<SearchResult> {
        self.s().search(&query, limit as usize).into_iter()
            .map(|h| SearchResult { snippet: h.snippet, chapter_title: h.chapter_title, position: h.position.into() })
            .collect()
    }
}
