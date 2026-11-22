//! 8-bit grayscale canvas with anti-aliased text (fontdue) and simple shapes.
//! 0 = black, 255 = white, matching the Kindle EPDC framebuffer.

use std::collections::HashMap;

pub const BLACK: u8 = 0;
pub const DARK: u8 = 64;
pub const MID: u8 = 128;
pub const LIGHT: u8 = 210;
pub const WHITE: u8 = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Rect { x0, y0, x1, y1 }
    }
    pub fn w(&self) -> i32 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> i32 {
        self.y1 - self.y0
    }
    pub fn cx(&self) -> i32 {
        (self.x0 + self.x1) / 2
    }
    pub fn cy(&self) -> i32 {
        (self.y0 + self.y1) / 2
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
    pub fn inset(&self, d: i32) -> Rect {
        Rect::new(self.x0 + d, self.y0 + d, self.x1 - d, self.y1 - d)
    }
    pub fn union(&self, o: &Rect) -> Rect {
        Rect::new(self.x0.min(o.x0), self.y0.min(o.y0), self.x1.max(o.x1), self.y1.max(o.y1))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Face {
    Sans,
    Bold,
    Mono,
    Italic,
}

/// Where (x, y) sits relative to the text: Left/Center/Right x Top/Middle/Baseline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Anchor {
    LT,
    LM,
    MM,
    RM,
    RT,
    MT,
}

/// One glyph of a laid-out string: `x` from the start of the string, `y` raised above the baseline.
#[derive(Clone, Copy)]
struct Placed {
    key: GlyphKey,
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
enum GlyphKey {
    Char(char),
    Shaped(usize, u16),
}

/// Scripts whose letters change shape and order (Devanagari, Bengali, Tamil, ...): they need
/// shaping, not one glyph per character. ZWJ/ZWNJ steer conjuncts.
fn needs_shaping(c: char) -> bool {
    matches!(c as u32, 0x0900..=0x0DFF | 0xA8E0..=0xA8FF | 0x200C | 0x200D)
}

struct Glyph {
    w: usize,
    h: usize,
    xmin: i32,
    ymin: i32,
    advance: f32,
    bitmap: Vec<u8>,
}

pub struct Fonts {
    sans: fontdue::Font,
    bold: fontdue::Font,
    mono: fontdue::Font,
    italic: fontdue::Font,
    cache: HashMap<(Face, char, u16), Glyph>,
    /// Glyphs of shaped (Indic) text, by (fallback font, glyph id, size).
    shaped_glyphs: HashMap<(usize, u16, u16), Glyph>,
    /// Shaped runs, so measuring and then drawing the same word shapes it once.
    shaped_runs: HashMap<(String, u16), Vec<Placed>>,
    /// Extra scripts (CJK, Devanagari, Arabic, Hebrew, Thai...), loaded on first use from disk.
    /// These use ab_glyph, which reads glyphs on demand: fontdue would turn every glyph of the
    /// 50k-glyph CJK font into outlines up front (~120 MB of RAM), enough to freeze the Kindle.
    fallbacks: Option<Vec<ab_glyph::FontVec>>,
}

/// Fallback fonts, in priority order (Alpine: font-droid-nonlatin; Debian: fonts-droid-fallback).
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

static SANS: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");
static BOLD: &[u8] = include_bytes!("../fonts/DejaVuSans-Bold.ttf");
static MONO: &[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf");
static ITALIC: &[u8] = include_bytes!("../fonts/DejaVuSans-Oblique.ttf");

/// Draw one glyph from a fallback font, in the same layout fontdue uses: `size` is pixels per em,
/// `ymin` is the bitmap's bottom edge above the baseline.
fn rasterize_fallback(font: &ab_glyph::FontVec, c: char, size: f32) -> Glyph {
    rasterize_id(font, ab_glyph::Font::glyph_id(font, c), size)
}

fn rasterize_id(font: &ab_glyph::FontVec, id: ab_glyph::GlyphId, size: f32) -> Glyph {
    use ab_glyph::{Font, ScaleFont};
    let upem = font.units_per_em().unwrap_or(1000.0);
    let scale = ab_glyph::PxScale::from(size * font.height_unscaled() / upem);
    let advance = font.as_scaled(scale).h_advance(id);
    let Some(outline) = font.outline_glyph(id.with_scale(scale)) else {
        return Glyph { w: 0, h: 0, xmin: 0, ymin: 0, advance, bitmap: Vec::new() };
    };
    let b = outline.px_bounds();
    let (w, h) = (b.width() as usize, b.height() as usize);
    let mut bitmap = vec![0u8; w * h];
    outline.draw(|x, y, cov| {
        let (x, y) = (x as usize, y as usize);
        if x < w && y < h {
            bitmap[y * w + x] = (cov.clamp(0.0, 1.0) * 255.0) as u8;
        }
    });
    Glyph { w, h, xmin: b.min.x as i32, ymin: -(b.max.y as i32), advance, bitmap }
}

fn run_advance(run: &[Placed]) -> f32 {
    run.last().map(|p| p.x).unwrap_or(0.0)
}

impl Fonts {
    pub fn load() -> Fonts {
        let s = fontdue::FontSettings::default();
        Fonts {
            sans: fontdue::Font::from_bytes(SANS, s).expect("sans font"),
            bold: fontdue::Font::from_bytes(BOLD, s).expect("bold font"),
            mono: fontdue::Font::from_bytes(MONO, s).expect("mono font"),
            italic: fontdue::Font::from_bytes(ITALIC, s).expect("italic font"),
            cache: HashMap::new(),
            fallbacks: None,
            shaped_glyphs: HashMap::new(),
            shaped_runs: HashMap::new(),
        }
    }

    fn font(&self, f: Face) -> &fontdue::Font {
        match f {
            Face::Sans => &self.sans,
            Face::Bold => &self.bold,
            Face::Mono => &self.mono,
            Face::Italic => &self.italic,
        }
    }

    fn load_fallbacks(&mut self) {
        if self.fallbacks.is_none() {
            self.fallbacks = Some(FALLBACK_FONTS.iter()
                .filter_map(|p| std::fs::read(p).ok())
                .filter_map(|b| ab_glyph::FontVec::try_from_vec(b).ok())
                .collect());
        }
    }

    fn glyph(&mut self, f: Face, c: char, size: u16) -> &Glyph {
        if !self.cache.contains_key(&(f, c, size)) {
            // missing in bold/mono -> regular face -> other scripts' fonts (loaded on first need)
            let need_fallback = self.font(f).lookup_glyph_index(c) == 0 && self.sans.lookup_glyph_index(c) == 0
                && !c.is_control() && c != ' ';
            if need_fallback {
                self.load_fallbacks();
            }
            let mut font = self.font(f);
            if font.lookup_glyph_index(c) == 0 && f != Face::Sans {
                font = &self.sans;
            }
            let fallback = if need_fallback {
                self.fallbacks.as_ref().and_then(|v| v.iter().find(|x| ab_glyph::Font::glyph_id(*x, c).0 != 0))
                    .map(|fb| rasterize_fallback(fb, c, size as f32))
            } else {
                None
            };
            let g = fallback.unwrap_or_else(|| {
                let (m, bitmap) = font.rasterize(c, size as f32);
                Glyph { w: m.width, h: m.height, xmin: m.xmin, ymin: m.ymin, advance: m.advance_width, bitmap }
            });
            if self.cache.len() > 4000 {
                self.cache.clear();
            }
            self.cache.insert((f, c, size), g);
        }
        &self.cache[&(f, c, size)]
    }

    /// (ascent, descent) in pixels; descent is negative.
    pub fn line_metrics(&self, f: Face, size: u16) -> (f32, f32) {
        match self.font(f).horizontal_line_metrics(size as f32) {
            Some(m) => (m.ascent, m.descent),
            None => (size as f32 * 0.8, -(size as f32) * 0.2),
        }
    }

    /// Position every glyph of `s`; returns the glyphs and the total advance. Indic runs are shaped
    /// with rustybuzz using the fallback font that covers them; everything else is one glyph per char.
    fn layout(&mut self, s: &str, f: Face, size: u16) -> (Vec<Placed>, f32) {
        let mut out = Vec::with_capacity(s.len());
        let mut pen = 0.0;
        let chars: Vec<(usize, char)> = s.char_indices().collect();
        let mut i = 0;
        while i < chars.len() {
            let (start, c) = chars[i];
            if needs_shaping(c) && !matches!(c, '\u{200C}' | '\u{200D}') {
                let mut j = i;
                while j < chars.len() && needs_shaping(chars[j].1) {
                    j += 1;
                }
                let end = chars.get(j).map(|x| x.0).unwrap_or(s.len());
                if let Some(run) = self.shape(&s[start..end], size) {
                    for p in &run {
                        out.push(Placed { key: p.key, x: pen + p.x, y: p.y });
                    }
                    // the run's advance is its trailing marker (see `shape`)
                    pen += run_advance(&run);
                    i = j;
                    continue;
                }
            }
            out.push(Placed { key: GlyphKey::Char(c), x: pen, y: 0.0 });
            pen += self.glyph(f, c, size).advance;
            i += 1;
        }
        out.retain(|p| !matches!(p.key, GlyphKey::Shaped(usize::MAX, _)));
        (out, pen)
    }

    /// Shape one Indic run; None when no installed font covers it.
    fn shape(&mut self, run: &str, size: u16) -> Option<Vec<Placed>> {
        if let Some(r) = self.shaped_runs.get(&(run.to_string(), size)) {
            return Some(r.clone());
        }
        self.load_fallbacks();
        let first = run.chars().find(|c| !matches!(c, '\u{200C}' | '\u{200D}'))?;
        let fonts = self.fallbacks.as_ref()?;
        let idx = fonts.iter().position(|x| ab_glyph::Font::glyph_id(x, first).0 != 0)?;
        let face = rustybuzz::Face::from_slice(fonts[idx].as_slice(), 0)?;
        let mut buf = rustybuzz::UnicodeBuffer::new();
        buf.push_str(run);
        buf.guess_segment_properties();
        let shaped = rustybuzz::shape(&face, &[], buf);
        let scale = size as f32 / face.units_per_em() as f32;
        let mut placed = Vec::new();
        let mut pen = 0.0;
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            placed.push(Placed {
                key: GlyphKey::Shaped(idx, info.glyph_id as u16),
                x: pen + pos.x_offset as f32 * scale,
                y: pos.y_offset as f32 * scale,
            });
            pen += pos.x_advance as f32 * scale;
        }
        // trailing marker carrying the run's total advance; dropped by `layout`
        placed.push(Placed { key: GlyphKey::Shaped(usize::MAX, 0), x: pen, y: 0.0 });
        if self.shaped_runs.len() > 2000 {
            self.shaped_runs.clear();
        }
        self.shaped_runs.insert((run.to_string(), size), placed.clone());
        Some(placed)
    }

    fn shaped_glyph(&mut self, idx: usize, id: u16, size: u16) -> &Glyph {
        if !self.shaped_glyphs.contains_key(&(idx, id, size)) {
            let g = match self.fallbacks.as_ref().and_then(|v| v.get(idx)) {
                Some(font) => rasterize_id(font, ab_glyph::GlyphId(id), size as f32),
                None => Glyph { w: 0, h: 0, xmin: 0, ymin: 0, advance: 0.0, bitmap: Vec::new() },
            };
            if self.shaped_glyphs.len() > 2000 {
                self.shaped_glyphs.clear();
            }
            self.shaped_glyphs.insert((idx, id, size), g);
        }
        &self.shaped_glyphs[&(idx, id, size)]
    }

    pub fn width(&mut self, s: &str, f: Face, size: u16) -> i32 {
        if s.chars().any(needs_shaping) {
            return self.layout(s, f, size).1.ceil() as i32;
        }
        let mut w = 0.0;
        for c in s.chars() {
            w += self.glyph(f, c, size).advance;
        }
        w.ceil() as i32
    }

    /// Advance of one monospace cell.
    pub fn mono_advance(&mut self, size: u16) -> f32 {
        self.glyph(Face::Mono, 'M', size).advance
    }

    /// Like `ellipsize`, but keeps the end of the text ("…/documents/books").
    pub fn ellipsize_start(&mut self, s: &str, f: Face, size: u16, max_w: i32) -> String {
        if self.width(s, f, size) <= max_w {
            return s.to_string();
        }
        let mut chars: Vec<char> = s.chars().collect();
        while !chars.is_empty() {
            chars.remove(0);
            let cand: String = "…".to_string() + &chars.iter().collect::<String>();
            if self.width(&cand, f, size) <= max_w {
                return cand;
            }
        }
        "…".into()
    }

    pub fn ellipsize(&mut self, s: &str, f: Face, size: u16, max_w: i32) -> String {
        if self.width(s, f, size) <= max_w {
            return s.to_string();
        }
        let mut chars: Vec<char> = s.chars().collect();
        while !chars.is_empty() {
            chars.pop();
            let cand: String = chars.iter().collect::<String>() + "…";
            if self.width(&cand, f, size) <= max_w {
                return cand;
            }
        }
        "…".into()
    }

    /// Word-wrap to lines no wider than max_w.
    pub fn wrap(&mut self, s: &str, f: Face, size: u16, max_w: i32) -> Vec<String> {
        let mut lines = Vec::new();
        for para in s.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let cand = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
                if self.width(&cand, f, size) <= max_w {
                    line = cand;
                } else {
                    if !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                    }
                    line = word.to_string();
                }
            }
            lines.push(line);
        }
        lines
    }
}

pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub buf: Vec<u8>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Canvas {
        Canvas { w, h, buf: vec![WHITE; (w * h) as usize] }
    }

    pub fn clear(&mut self, v: u8) {
        self.buf.fill(v);
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, color: u8, alpha: u8) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || alpha == 0 {
            return;
        }
        let i = (y * self.w + x) as usize;
        let a = alpha as u32;
        self.buf[i] = ((self.buf[i] as u32 * (255 - a) + color as u32 * a) / 255) as u8;
    }

    pub fn fill(&mut self, r: Rect, v: u8) {
        let x0 = r.x0.clamp(0, self.w);
        let x1 = r.x1.clamp(0, self.w);
        let y0 = r.y0.clamp(0, self.h);
        let y1 = r.y1.clamp(0, self.h);
        if x1 <= x0 {
            return;
        }
        for y in y0..y1 {
            let s = (y * self.w + x0) as usize;
            self.buf[s..s + (x1 - x0) as usize].fill(v);
        }
    }

    pub fn hline(&mut self, x0: i32, x1: i32, y: i32, width: i32, v: u8) {
        self.fill(Rect::new(x0, y, x1, y + width), v);
    }

    pub fn vline(&mut self, x: i32, y0: i32, y1: i32, width: i32, v: u8) {
        self.fill(Rect::new(x, y0, x + width, y1), v);
    }

    pub fn outline(&mut self, r: Rect, width: i32, v: u8) {
        self.hline(r.x0, r.x1, r.y0, width, v);
        self.hline(r.x0, r.x1, r.y1 - width, width, v);
        self.vline(r.x0, r.y0, r.y1, width, v);
        self.vline(r.x1 - width, r.y0, r.y1, width, v);
    }

    /// Filled rounded rectangle with anti-aliased corners.
    pub fn round_fill(&mut self, r: Rect, radius: i32, v: u8) {
        let rad = radius.min(r.w() / 2).min(r.h() / 2).max(0);
        if rad == 0 {
            self.fill(r, v);
            return;
        }
        self.fill(Rect::new(r.x0, r.y0 + rad, r.x1, r.y1 - rad), v);
        let rf = rad as f32;
        for dy in 0..rad {
            // distance from the corner circle's center row
            let fy = rf - dy as f32 - 0.5;
            let span = (rf * rf - fy * fy).max(0.0).sqrt();
            let inset = rf - span;
            let full = inset.ceil() as i32;
            let frac = (full as f32 - inset).clamp(0.0, 1.0);
            for &y in &[r.y0 + dy, r.y1 - 1 - dy] {
                self.fill(Rect::new(r.x0 + full, y, r.x1 - full, y + 1), v);
                let a = (frac * 255.0) as u8;
                self.blend(r.x0 + full - 1, y, v, a);
                self.blend(r.x1 - full, y, v, a);
            }
        }
    }

    pub fn round_rect(&mut self, r: Rect, radius: i32, fill: u8, border: i32, border_color: u8) {
        if border > 0 {
            self.round_fill(r, radius, border_color);
            self.round_fill(r.inset(border), (radius - border).max(0), fill);
        } else {
            self.round_fill(r, radius, fill);
        }
    }

    /// Draw text; returns its width.
    pub fn text(&mut self, fonts: &mut Fonts, x: i32, y: i32, s: &str, face: Face, size: u16, color: u8,
                anchor: Anchor) -> i32 {
        let w = fonts.width(s, face, size);
        let (asc, desc) = fonts.line_metrics(face, size);
        let left = match anchor {
            Anchor::LT | Anchor::LM => x,
            Anchor::MM | Anchor::MT => x - w / 2,
            Anchor::RM | Anchor::RT => x - w,
        } as f32;
        let baseline = match anchor {
            Anchor::LT | Anchor::RT | Anchor::MT => y as f32 + asc,
            Anchor::LM | Anchor::MM | Anchor::RM => y as f32 + (asc + desc) / 2.0,
        };
        let placed = if s.chars().any(needs_shaping) {
            fonts.layout(s, face, size).0
        } else {
            Vec::new()
        };
        let mut chars = s.chars();
        let mut k = 0;
        let mut pen = left;
        loop {
            // plain text walks the chars; shaped text walks the laid-out glyphs
            let (g, x, raise) = if placed.is_empty() {
                let Some(c) = chars.next() else { break };
                (fonts.glyph(face, c, size), pen, 0.0)
            } else {
                let Some(p) = placed.get(k) else { break };
                k += 1;
                let g = match p.key {
                    GlyphKey::Char(c) => fonts.glyph(face, c, size),
                    GlyphKey::Shaped(idx, id) => fonts.shaped_glyph(idx, id, size),
                };
                (g, left + p.x, p.y)
            };
            let gx = (x + g.xmin as f32).round() as i32;
            let gy = (baseline - raise - g.ymin as f32 - g.h as f32).round() as i32;
            let (gw, gh) = (g.w, g.h);
            // copy out to avoid holding a borrow on fonts while drawing
            let bitmap = &g.bitmap;
            for row in 0..gh {
                let py = gy + row as i32;
                if py < 0 || py >= self.h {
                    continue;
                }
                for col in 0..gw {
                    let a = bitmap[row * gw + col];
                    if a != 0 {
                        let px = gx + col as i32;
                        if px >= 0 && px < self.w {
                            let i = (py * self.w + px) as usize;
                            let aa = a as u32;
                            self.buf[i] = ((self.buf[i] as u32 * (255 - aa) + color as u32 * aa) / 255) as u8;
                        }
                    }
                }
            }
            pen += g.advance;
        }
        w
    }

    /// Bounding box of pixels that differ from `other` (same size), or None.
    pub fn diff_bbox(&self, other: &Canvas) -> Option<Rect> {
        let (mut x0, mut y0, mut x1, mut y1) = (self.w, self.h, -1, -1);
        for y in 0..self.h {
            let s = (y * self.w) as usize;
            let a = &self.buf[s..s + self.w as usize];
            let b = &other.buf[s..s + self.w as usize];
            if a == b {
                continue;
            }
            let first = a.iter().zip(b).position(|(p, q)| p != q).unwrap() as i32;
            let last = (self.w - 1) - a.iter().rev().zip(b.iter().rev()).position(|(p, q)| p != q).unwrap() as i32;
            x0 = x0.min(first);
            x1 = x1.max(last);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        if x1 < 0 {
            None
        } else {
            Some(Rect::new(x0, y0, x1 + 1, y1 + 1))
        }
    }

    pub fn copy_from(&mut self, other: &Canvas) {
        self.buf.copy_from_slice(&other.buf);
    }

    /// Paste a grayscale image (row-major) at (x, y).
    pub fn blit(&mut self, img: &[u8], iw: i32, ih: i32, x: i32, y: i32) {
        for row in 0..ih {
            let py = y + row;
            if py < 0 || py >= self.h {
                continue;
            }
            for col in 0..iw {
                let px = x + col;
                if px >= 0 && px < self.w {
                    self.buf[(py * self.w + px) as usize] = img[(row * iw + col) as usize];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rss_kb() -> usize {
        std::fs::read_to_string("/proc/self/status").ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
            .unwrap_or(0)
    }

    #[test]
    fn fallback_scripts_render_without_eating_memory() {
        let mut f = Fonts::load();
        let before = rss_kb();
        for c in ['中', 'あ', '한', 'ह', '🎉'] {
            let g = f.glyph(Face::Sans, c, 32);
            let present = FALLBACK_FONTS.iter().any(|p| std::path::Path::new(p).exists());
            if c != '🎉' && present {
                assert!(g.w > 4 && g.h > 4 && g.bitmap.iter().any(|&v| v > 128), "{c} drew nothing");
                assert!(g.ymin > -16 && g.advance > 16.0, "{c}: ymin {} advance {}", g.ymin, g.advance);
            }
        }
        let grown = rss_kb().saturating_sub(before);
        assert!(grown < 20_000, "fallback fonts took {grown} kB");
    }

    #[test]
    fn hindi_is_shaped() {
        if !std::path::Path::new(FALLBACK_FONTS[2]).exists() {
            return; // no Devanagari font in this environment
        }
        let mut f = Fonts::load();
        // क्ष is one conjunct glyph, not three
        let (glyphs, w) = f.layout("क्ष", Face::Sans, 32);
        assert!(glyphs.len() < 3 && w > 10.0, "{} glyphs", glyphs.len());
        // कि: the vowel sign is drawn first (left of the consonant)
        let (glyphs, _) = f.layout("कि", Face::Sans, 32);
        let ka = f.layout("क", Face::Sans, 32).0[0].key;
        assert!(matches!((glyphs[0].key, ka), (GlyphKey::Shaped(_, a), GlyphKey::Shaped(_, b)) if a != b));
        // measuring a page of words stays quick (shaped runs are cached)
        let t = std::time::Instant::now();
        let words: Vec<String> = (0..300).map(|i| format!("किताब{} प्रेम{}", i % 40, i % 7)).collect();
        for w in &words {
            f.width(w, Face::Sans, 24);
        }
        eprintln!("300 words shaped in {:?}", t.elapsed());
    }
}
