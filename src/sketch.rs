//! Sketchpad (finger drawing with fast e-ink updates, saved as PNG) and a PNG image viewer.

use crate::canvas::*;
use crate::sys::{self, Sys};
use crate::ui::*;

const TOOLBAR_H: i32 = 62;

pub struct Sketch {
    w: i32,
    h: i32,
    ink: Vec<u8>, // drawing surface (grayscale), sized to the content area below the toolbar
    pen: i32,
    eraser: bool,
    last: Option<(i32, i32)>,
    origin: (i32, i32),
    dirty: bool,
}

const SK_THIN: u32 = 1;
const SK_THICK: u32 = 2;
const SK_ERASE: u32 = 3;
const SK_CLEAR: u32 = 4;
const SK_SAVE: u32 = 5;
const T_CLEAR: u32 = 1;

impl Sketch {
    pub fn new() -> Sketch {
        Sketch { w: 0, h: 0, ink: vec![], pen: 2, eraser: false, last: None, origin: (0, HEADER_H + TOOLBAR_H), dirty: false }
    }

    fn dot(&mut self, x: i32, y: i32) {
        let r = if self.eraser { 14 } else { self.pen };
        let v = if self.eraser { WHITE } else { BLACK };
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    let (px, py) = (x + dx, y + dy);
                    if px >= 0 && py >= 0 && px < self.w && py < self.h {
                        self.ink[(py * self.w + px) as usize] = v;
                    }
                }
            }
        }
    }

    fn line(&mut self, (x0, y0): (i32, i32), (x1, y1): (i32, i32)) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        for i in 0..=steps {
            let x = x0 + (x1 - x0) * i / steps;
            let y = y0 + (y1 - y0) * i / steps;
            self.dot(x, y);
        }
    }

    fn save(&self, sys: &Sys) -> Result<String, String> {
        let dir = format!("{}/sketches", sys.us);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = format!("{dir}/sketch-{}.png", sys::clock("%Y%m%d-%H%M%S"));
        let f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&self.ink).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

impl App for Sketch {
    /// Leaving with ‹ or ⌂ keeps the drawing (saved like the Save button would).
    fn on_hide(&mut self, sys: &mut Sys) {
        if self.dirty && self.save(sys).is_ok() {
            self.dirty = false;
        }
    }

    fn title(&self) -> String {
        "Sketch".into()
    }

    fn wants_raw_touch(&self) -> bool {
        true
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let (w, h) = (a.w(), a.h() - TOOLBAR_H);
        if self.w != w || self.h != h {
            self.w = w;
            self.h = h;
            self.ink = vec![WHITE; (w * h) as usize];
            self.origin = (a.x0, a.y0 + TOOLBAR_H);
        }
        let b = Btn::size(18);
        let items = [(SK_THIN, "Thin", !self.eraser && self.pen == 2), (SK_THICK, "Thick", !self.eraser && self.pen == 5),
                     (SK_ERASE, "Eraser", self.eraser), (SK_CLEAR, "Clear", false), (SK_SAVE, "Save", false)];
        let bw = (a.w() - 12) / items.len() as i32;
        for (i, (id, label, on)) in items.iter().enumerate() {
            let x = a.x0 + 6 + i as i32 * bw;
            cx.button(Rect::new(x + 3, a.y0 + 8, x + bw - 3, a.y0 + 54), label, Some(Hit::App(*id)), Btn { inverted: *on, ..b });
        }
        cx.c.hline(a.x0, a.x1, a.y0 + TOOLBAR_H - 2, 2, BLACK);
        cx.c.blit(&self.ink, self.w, self.h, self.origin.0, self.origin.1);
    }

    fn raw_touch(&mut self, ev: RawTouch, _sys: &mut Sys) -> Cmd {
        let to_local = |x: i32, y: i32, o: (i32, i32)| (x - o.0, y - o.1);
        match ev {
            RawTouch::Down(x, y) => {
                let p = to_local(x, y, self.origin);
                if p.1 < 0 {
                    return Cmd::None; // toolbar: handled as a normal tap
                }
                self.dot(p.0, p.1);
                self.last = Some(p);
            }
            RawTouch::Move(x, y) => {
                let p = to_local(x, y, self.origin);
                if let Some(l) = self.last {
                    if p.1 >= 0 {
                        self.line(l, p);
                    }
                }
                self.last = Some(p);
            }
            RawTouch::Up(_, _) => self.last = None,
        }
        self.dirty = true;
        Cmd::Fast
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        match id {
            SK_THIN => {
                self.pen = 2;
                self.eraser = false;
            }
            SK_THICK => {
                self.pen = 5;
                self.eraser = false;
            }
            SK_ERASE => self.eraser = true,
            SK_CLEAR => return Cmd::Dialog(Dialog::confirm(T_CLEAR, "Clear the page?", "", "Clear")),
            SK_SAVE => {
                return match self.save(sys) {
                    Ok(p) => {
                        self.dirty = false;
                        Cmd::Toast(format!("Saved {}", p.rsplit('/').next().unwrap_or("")))
                    }
                    Err(e) => Cmd::Toast(e),
                };
            }
            _ => {}
        }
        Cmd::Fast
    }

    fn dialog(&mut self, tag: u32, choice: usize, _t: String, _sys: &mut Sys) -> Cmd {
        if tag == T_CLEAR && choice != CANCEL {
            self.ink.fill(WHITE);
            return Cmd::Full;
        }
        Cmd::None
    }

    fn raw_top(&self) -> i32 {
        HEADER_H + TOOLBAR_H
    }
}

/// View a PNG file (sketches, screenshots), scaled to fit and converted to grayscale.
pub struct ImageView {
    path: String,
    img: Option<(Vec<u8>, i32, i32)>,
    error: Option<String>,
}

impl ImageView {
    pub fn new(path: &str) -> ImageView {
        ImageView { path: path.into(), img: None, error: None }
    }

    fn load(&mut self, max_w: i32, max_h: i32) {
        let r = (|| -> Result<(Vec<u8>, i32, i32), String> {
            let f = std::fs::File::open(&self.path).map_err(|e| e.to_string())?;
            let mut dec = png::Decoder::new(std::io::BufReader::new(f));
            dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
            let mut rd = dec.read_info().map_err(|e| e.to_string())?;
            let mut buf = vec![0; rd.output_buffer_size()];
            let info = rd.next_frame(&mut buf).map_err(|e| e.to_string())?;
            let (w, h) = (info.width as i32, info.height as i32);
            let ch = match info.color_type {
                png::ColorType::Grayscale => 1,
                png::ColorType::GrayscaleAlpha => 2,
                png::ColorType::Rgb => 3,
                png::ColorType::Rgba => 4,
                png::ColorType::Indexed => return Err("indexed PNG not supported".into()),
            };
            let gray: Vec<u8> = buf.chunks(ch).take((w * h) as usize).map(|p| match ch {
                1 | 2 => p[0],
                _ => ((p[0] as u32 * 30 + p[1] as u32 * 59 + p[2] as u32 * 11) / 100) as u8,
            }).collect();
            // nearest-neighbour scale to fit
            let s = (max_w as f32 / w as f32).min(max_h as f32 / h as f32).min(1.0);
            let (nw, nh) = (((w as f32 * s) as i32).max(1), ((h as f32 * s) as i32).max(1));
            let mut out = vec![WHITE; (nw * nh) as usize];
            for y in 0..nh {
                for x in 0..nw {
                    let sx = ((x as f32 / s) as i32).min(w - 1);
                    let sy = ((y as f32 / s) as i32).min(h - 1);
                    out[(y * nw + x) as usize] = gray[(sy * w + sx) as usize];
                }
            }
            Ok((out, nw, nh))
        })();
        match r {
            Ok(i) => self.img = Some(i),
            Err(e) => self.error = Some(e),
        }
    }
}

impl App for ImageView {
    fn title(&self) -> String {
        self.path.rsplit('/').next().unwrap_or("Image").to_string()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        if self.img.is_none() && self.error.is_none() {
            self.load(a.w(), a.h());
        }
        match (&self.img, &self.error) {
            (Some((px, w, h)), _) => cx.c.blit(px, *w, *h, a.x0 + (a.w() - w) / 2, a.y0 + (a.h() - h) / 2),
            (_, Some(e)) => {
                cx.text(a.cx(), a.cy(), &format!("Can't show this image: {e}"), 18, BLACK, Anchor::MM);
            }
            _ => {}
        }
    }
}
