//! The platform-neutral core shared by every build (e-ink devices, Android, and later iOS and desktop):
//! book formats, fonts and text shaping, hyphenation, pagination, and drawing pages into a
//! grayscale pixel buffer. Nothing here touches hardware or assumes a filesystem layout.

pub mod books;
pub mod canvas;
pub mod epub;
pub mod feed;
pub mod formats;
pub mod hyph;
pub mod image;
pub mod json;
pub mod layout;
pub mod mobi;
pub mod session;

/// What a library screen shows for a book file, without laying it out.
pub struct BookInfo {
    pub title: String,
    pub author: String,
    /// the cover as stored in the book (JPEG, PNG or GIF bytes), if it has one
    pub cover: Option<Vec<u8>>,
}

/// Read a book's title, author and cover.
pub fn book_info(path: &str) -> Result<BookInfo, String> {
    let lower = path.to_lowercase();
    let mut book = epub::Book::open(path)?;
    let cover = if [".mobi", ".azw3", ".azw", ".prc"].iter().any(|e| lower.ends_with(e)) {
        mobi::cover(path)
    } else {
        book.cover_image()
    };
    Ok(BookInfo { title: book.title.clone(), author: book.author.clone(), cover })
}

/// The fonts the engine's own tests draw with: the Kindle build's DejaVu faces (the engine bundles
/// no fonts; each platform supplies its own).
#[cfg(test)]
pub(crate) fn test_fonts() -> canvas::Fonts {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../platforms/kindle/fonts/");
    let read = |f: &str| std::fs::read(format!("{dir}{f}")).expect("Kindle fonts for tests");
    let fallbacks = ["/usr/share/fonts/droid-nonlatin/DroidSansFallbackFull.ttf",
                     "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
                     "/usr/share/fonts/droid-nonlatin/DroidSansDevanagari-Regular.ttf"].iter().map(|s| s.to_string()).collect();
    canvas::Fonts::new(&read("DejaVuSans.ttf"), &read("DejaVuSans-Bold.ttf"), &read("DejaVuSans-Oblique.ttf"),
                       &read("DejaVuSansMono.ttf"), fallbacks).unwrap()
}
