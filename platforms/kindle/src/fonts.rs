//! The fonts this build draws with: DejaVu, embedded from `platforms/kindle/fonts/`, and the
//! non-Latin fonts the Alpine image installs, for scripts DejaVu lacks.

use crate::canvas::Fonts;

static SANS: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");
static BOLD: &[u8] = include_bytes!("../fonts/DejaVuSans-Bold.ttf");
static ITALIC: &[u8] = include_bytes!("../fonts/DejaVuSans-Oblique.ttf");
static MONO: &[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf");

/// Fallback fonts, in priority order (Alpine: font-droid-nonlatin; Debian, for the simulator:
/// fonts-droid-fallback).
const FALLBACK_FONTS: &[&str] = &[
    "/usr/share/fonts/droid-nonlatin/DroidSansFallbackFull.ttf",
    "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    "/usr/share/fonts/droid-nonlatin/DroidSansDevanagari-Regular.ttf",
    "/usr/share/fonts/droid-nonlatin/DroidNaskh-Regular.ttf",
    "/usr/share/fonts/droid-nonlatin/DroidSansHebrew-Regular.ttf",
    "/usr/share/fonts/droid-nonlatin/DroidSansThai.ttf",
    "/usr/share/fonts/droid-nonlatin/DroidSansArmenian.ttf",
    "/usr/share/fonts/droid-nonlatin/DroidSansGeorgian.ttf",
    "/usr/share/fonts/droid-nonlatin/DroidSansEthiopic-Regular.ttf",
];

pub fn load() -> Fonts {
    Fonts::new(SANS, BOLD, ITALIC, MONO, FALLBACK_FONTS.iter().map(|p| p.to_string()).collect())
        .expect("embedded DejaVu fonts")
}
