//! Page layout: turns a chapter's blocks into pages of positioned lines (with justification and
//! hyphenation) and draws a page onto a canvas. Positions are character offsets into the chapter,
//! so a reading position survives changes of font size, margins or screen size.

use crate::canvas::*;
use crate::epub::{Block, BlockKind};
use std::collections::HashMap;
use std::sync::Arc as Rc;

/// Reader layout preferences (margins, line spacing, hyphenation).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Layout {
    /// left and right margin, in pixels
    pub margin: i32,
    /// space above the first line and below the last, in pixels (room for page chrome)
    pub top: i32,
    pub bottom: i32,
    pub spacing: i32, // line height in % of the font size
    pub hyphenate: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { margin: 36, top: 34, bottom: 40, spacing: 145, hyphenate: true }
    }
}

pub const MARGINS: [(&str, i32); 3] = [("Narrow", 20), ("Normal", 36), ("Wide", 60)];
pub const SPACINGS: [(&str, i32); 3] = [("Compact", 130), ("Normal", 145), ("Relaxed", 165)];


struct Word {
    text: String,
    face: Face,
    w: i32,
    offset: usize,
    link: Option<u32>,
    /// no space before it in the text ("paper" + "." when the link ends before the full stop):
    /// drawn without a gap and kept on the same line as the word before
    glue: bool,
}

pub struct Line {
    words: Vec<Word>,
    img: Option<Rc<crate::image::Gray>>,
    lh: i32,
    y: i32,
    x0: i32,
    avail: i32,
    size: u16,
    justify: bool,
    center: bool,
    rule: bool,
}

pub struct Page {
    lines: Vec<Line>,
    pub start: usize,
}

struct Style {
    size: u16,
    face: Face,
    center: bool,
    indent: i32,
    left: i32,
    before: i32,
    after: i32,
    justify: bool,
    prefix: &'static str,
}

fn style(kind: BlockKind, fs: u16) -> Style {
    let f = fs as i32;
    let base = Style { size: fs, face: Face::Sans, center: false, indent: f * 3 / 2, left: 0, before: 0,
                       after: f * 2 / 5, justify: true, prefix: "" };
    match kind {
        BlockKind::Para => base,
        BlockKind::Heading(l) => {
            let size = match l { 1 => fs * 3 / 2, 2 => fs * 13 / 10, _ => fs * 11 / 10 };
            Style { size, face: Face::Bold, center: l <= 2, indent: 0, before: f, after: f * 3 / 5, justify: false, ..base }
        }
        BlockKind::Item => Style { indent: 0, left: f, justify: false, prefix: "• ", after: f / 4, ..base },
        BlockKind::Quote => Style { left: f * 3 / 2, indent: 0, size: fs * 19 / 20, ..base },
        BlockKind::Pre => Style { face: Face::Mono, size: fs * 17 / 20, indent: 0, justify: false, ..base },
        BlockKind::Rule | BlockKind::Image => Style { indent: 0, before: f / 2, after: f / 2, justify: false, ..base },
    }
}

/// Lay out a chapter's blocks into pages of the given area.
/// Decoded, page-fitted images of a chapter, keyed by zip path.
pub type Images = HashMap<String, Rc<crate::image::Gray>>;

pub fn paginate(blocks: &[Block], fonts: &mut Fonts, fs: u16, area: Rect, lay: Layout, images: &Images) -> Vec<Page> {
    let mut pages = vec![Page { lines: vec![], start: 0 }];
    let top = area.y0 + lay.top;
    let bottom = area.y1 - lay.bottom;
    let margin = lay.margin;
    let width = area.w() - 2 * margin;
    let mut y = top;
    for (bi, b) in blocks.iter().enumerate() {
        let st = style(b.kind, fs);
        let line_h = st.size as i32 * lay.spacing / 100;
        if y > top {
            y += st.before;
        }
        if b.kind == BlockKind::Image {
            let Some(img) = images.get(&b.runs[0].text) else { continue };
            let h = img.h + 12;
            if y > top && y + h > bottom {
                pages.push(Page { lines: vec![], start: b.start });
                y = top;
            }
            let x0 = area.x0 + (area.w() - img.w) / 2;
            pages.last_mut().unwrap().lines.push(Line { words: vec![], img: Some(img.clone()), lh: h, y, x0, avail: img.w,
                size: st.size, justify: false, center: false, rule: false });
            y += h + 8;
            continue;
        }
        if b.kind == BlockKind::Rule {
            if y + line_h > bottom {
                pages.push(Page { lines: vec![], start: b.start });
                y = top;
            }
            pages.last_mut().unwrap().lines.push(Line { words: vec![], img: None, lh: line_h, y, x0: area.x0 + margin, avail: width,
                size: st.size, justify: false, center: false, rule: true });
            y += line_h;
            continue;
        }
        // headings: don't strand them at the bottom of a page
        if matches!(b.kind, BlockKind::Heading(_)) && y > top && y + line_h * 3 > bottom {
            pages.push(Page { lines: vec![], start: b.start });
            y = top;
        }
        // words
        let mut words: Vec<Word> = vec![];
        let mut off = b.start;
        if !st.prefix.is_empty() {
            words.push(Word { text: st.prefix.trim_end().into(), face: st.face, w: fonts.width(st.prefix.trim_end(), st.face, st.size), offset: off, link: None, glue: false });
        }
        let mut after_space = true;
        for run in &b.runs {
            let face = if st.face == Face::Mono { Face::Mono } else if run.bold { Face::Bold }
                       else if run.italic { Face::Italic } else { st.face };
            for (i, w) in run.text.split(' ').enumerate() {
                if i > 0 {
                    off += 1;
                }
                if w.is_empty() {
                    continue;
                }
                let ww = fonts.width(w, face, st.size);
                let glue = i == 0 && !after_space && !words.is_empty();
                words.push(Word { text: w.to_string(), face, w: ww, offset: off, link: run.link, glue });
                off += w.chars().count();
            }
            if !run.text.is_empty() {
                after_space = run.text.ends_with(' ');
            }
        }
        let space = fonts.width(" ", st.face, st.size).max(3);
        // list items: wrapped lines hang under the text, not the bullet
        let hang = if st.prefix.is_empty() { 0 } else { fonts.width(st.prefix, st.face, st.size) };
        let mut i = 0;
        let mut first = true;
        while i < words.len() {
            let indent = if first && st.indent > 0 && bi > 0 { st.indent } else if !first { hang } else { 0 };
            let x0 = area.x0 + margin + st.left + indent;
            let avail = width - st.left - indent;
            let mut line_words = vec![];
            let mut used = 0;
            while i < words.len() {
                let w = &words[i];
                // the word plus any fragments glued to it move to the next line together
                let mut k = i + 1;
                let mut cluster = w.w;
                while k < words.len() && words[k].glue {
                    cluster += words[k].w;
                    k += 1;
                }
                let gap = if line_words.is_empty() || w.glue { 0 } else { space };
                if used + gap + cluster <= avail || line_words.is_empty() && w.w <= avail {
                    let end = if used + gap + cluster <= avail { k } else { i + 1 };
                    used += gap;
                    for j in i..end {
                        used += words[j].w;
                        line_words.push(std::mem::replace(&mut words[j], Word { text: String::new(), face: st.face, w: 0, offset: 0, link: None, glue: false }));
                    }
                    i = end;
                } else if line_words.is_empty() {
                    // a single word wider than the line: split it by characters
                    let chars: Vec<char> = w.text.chars().collect();
                    let mut cut = chars.len();
                    while cut > 1 && fonts.width(&chars[..cut].iter().collect::<String>(), w.face, st.size) > avail {
                        cut -= 1;
                    }
                    let head: String = chars[..cut].iter().collect();
                    let tail: String = chars[cut..].iter().collect();
                    let (face, offset, link) = (w.face, w.offset, w.link);
                    let hw = fonts.width(&head, face, st.size);
                    let tw = fonts.width(&tail, face, st.size);
                    line_words.push(Word { text: head, face, w: hw, offset, link, glue: false });
                    words[i] = Word { text: tail, face, w: tw, offset: offset + cut, link, glue: false };
                    break;
                } else {
                    // hyphenate the word that doesn't fit, if part of it does
                    if lay.hyphenate && st.justify {
                        let room = avail - used - if w.glue { 0 } else { space };
                        let core: &str = w.text.trim_end_matches(|c: char| !c.is_alphanumeric());
                        let chars: Vec<char> = core.chars().collect();
                        for &p in crate::hyph::breaks(core).iter().rev() {
                            let head: String = chars[..p].iter().collect::<String>() + "-";
                            let hw = fonts.width(&head, w.face, st.size);
                            if hw <= room {
                                let tail: String = w.text.chars().skip(p).collect();
                                let (face, offset, link) = (w.face, w.offset, w.link);
                                let tw = fonts.width(&tail, face, st.size);
                                let glue = w.glue;
                                line_words.push(Word { text: head, face, w: hw, offset, link, glue });
                                words[i] = Word { text: tail, face, w: tw, offset: offset + p, link, glue: false };
                                break;
                            }
                        }
                    }
                    break;
                }
            }
            if y + line_h > bottom {
                let start = line_words.first().map_or(b.start, |w| w.offset);
                pages.push(Page { lines: vec![], start });
                y = top;
            }
            let last_line = i >= words.len();
            pages.last_mut().unwrap().lines.push(Line { words: line_words, img: None, lh: line_h, y, x0, avail, size: st.size,
                justify: st.justify && !last_line, center: st.center, rule: false });
            y += line_h;
            first = false;
        }
        y += st.after;
    }
    pages.retain(|p| !p.lines.is_empty());
    if pages.is_empty() {
        pages.push(Page { lines: vec![], start: 0 });
    }
    pages
}

/// Where things ended up on screen after drawing a page.
#[derive(Default, Clone)]
pub struct PageGeom {
    pub links: Vec<(Rect, u32)>,
    pub words: Vec<(Rect, usize, String)>, // (box, chapter offset, text)
    /// where pictures were drawn
    pub images: Vec<Rect>,
}

/// Draw a page; returns link areas (rect, link index) and word boxes.
pub fn draw_page(c: &mut Canvas, f: &mut Fonts, page: &Page) -> PageGeom {
    let mut links: Vec<(Rect, u32)> = vec![];
    let mut words: Vec<(Rect, usize, String)> = vec![];
    let mut images: Vec<Rect> = vec![];
    for line in &page.lines {
        if let Some(img) = &line.img {
            c.blit(&img.px, img.w, img.h, line.x0, line.y);
            images.push(Rect::new(line.x0, line.y, line.x0 + img.w, line.y + img.h));
            continue;
        }
        if line.rule {
            let w = line.avail / 4;
            let x = line.x0 + (line.avail - w) / 2;
            c.hline(x, x + w, line.y + line.size as i32 / 2, 1, DARK);
            continue;
        }
        let n = line.words.len() as i32;
        if n == 0 {
            continue;
        }
        let space = f.width(" ", Face::Sans, line.size).max(3);
        let gaps = line.words.iter().skip(1).filter(|w| !w.glue).count() as i32;
        let total: i32 = line.words.iter().map(|w| w.w).sum::<i32>() + space * gaps;
        let mut gap = space as f32;
        let mut x = line.x0 as f32;
        if line.center {
            x = (line.x0 + (line.avail - total) / 2) as f32;
        } else if line.justify && gaps > 0 {
            let g = (line.avail - (total - space * gaps)) as f32 / gaps as f32;
            // justify unless the gaps would be absurd (very few words on a wide line)
            if g <= space as f32 * 4.0 || line.size >= 24 && g <= space as f32 * 6.0 {
                gap = g;
            }
        }
        let lh = line.lh;
        for (j, w) in line.words.iter().enumerate() {
            if j > 0 && !w.glue {
                x += gap;
            }
            let wx = x.round() as i32;
            c.text(f, wx, line.y, &w.text, w.face, line.size, BLACK, Anchor::LT);
            words.push((Rect::new(wx - 2, line.y - 4, wx + w.w + 2, line.y + lh - 2), w.offset, w.text.clone()));
            if let Some(l) = w.link {
                let uy = line.y + line.size as i32 * 118 / 100;
                c.hline(wx, wx + w.w, uy, 1, BLACK);
                let r = Rect::new(wx - 3, line.y - 4, wx + w.w + 3, line.y + lh - 2);
                // merge with the previous word of the same link on this line
                match links.last_mut() {
                    Some((pr, pl)) if *pl == l && pr.y0 == r.y0 && r.x0 - pr.x1 < 40 => pr.x1 = r.x1,
                    _ => links.push((r, l)),
                }
            }
            x += w.w as f32;
        }
    }
    PageGeom { links, words, images }
}
