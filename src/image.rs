//! Image decoding (PNG, JPEG) to 8-bit grayscale, and quality scaling for e-ink.

use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;

pub struct Gray {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
}

/// Decode PNG or JPEG bytes into grayscale.
pub fn decode(bytes: &[u8]) -> Result<Gray, String> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        let opts = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::Luma);
        let mut d = zune_jpeg::JpegDecoder::new_with_options(bytes, opts);
        let px = d.decode().map_err(|e| format!("JPEG: {e:?}"))?;
        let info = d.info().ok_or("JPEG: no size")?;
        let (w, h) = (info.width as i32, info.height as i32);
        if px.len() < (w * h) as usize {
            return Err("JPEG: short image".into());
        }
        Ok(Gray { w, h, px })
    } else {
        Err("unsupported image format".into())
    }
}

fn decode_png(bytes: &[u8]) -> Result<Gray, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
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
    let px = buf.chunks(ch).take((w * h) as usize).map(|p| match ch {
        1 => p[0],
        2 => blend_white(p[0], p[1]),
        3 => luma(p[0], p[1], p[2]),
        _ => blend_white(luma(p[0], p[1], p[2]), p[3]),
    }).collect();
    Ok(Gray { w, h, px })
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 30 + g as u32 * 59 + b as u32 * 11) / 100) as u8
}

/// Transparent pixels go to white (the page colour).
fn blend_white(v: u8, a: u8) -> u8 {
    ((v as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8
}

/// Resize with area averaging (good for downscaling covers and photos).
pub fn resize(src: &Gray, nw: i32, nh: i32) -> Gray {
    let (nw, nh) = (nw.max(1), nh.max(1));
    let mut px = vec![255u8; (nw * nh) as usize];
    let sx = src.w as f32 / nw as f32;
    let sy = src.h as f32 / nh as f32;
    for y in 0..nh {
        let y0 = (y as f32 * sy) as i32;
        let y1 = (((y + 1) as f32 * sy) as i32).max(y0 + 1).min(src.h);
        for x in 0..nw {
            let x0 = (x as f32 * sx) as i32;
            let x1 = (((x + 1) as f32 * sx) as i32).max(x0 + 1).min(src.w);
            let mut sum = 0u32;
            let mut n = 0u32;
            for yy in y0..y1 {
                let row = (yy * src.w) as usize;
                for xx in x0..x1 {
                    sum += src.px[row + xx as usize] as u32;
                    n += 1;
                }
            }
            px[(y * nw + x) as usize] = if n > 0 { (sum / n) as u8 } else { 255 };
        }
    }
    Gray { w: nw, h: nh, px }
}

/// Scale to fit inside (max_w, max_h), keeping the aspect ratio.
pub fn fit(src: &Gray, max_w: i32, max_h: i32) -> Gray {
    let s = (max_w as f32 / src.w as f32).min(max_h as f32 / src.h as f32);
    resize(src, (src.w as f32 * s) as i32, (src.h as f32 * s) as i32)
}

/// Scale to cover (w, h) exactly, cropping the overflow (centered).
pub fn fill(src: &Gray, w: i32, h: i32) -> Gray {
    let s = (w as f32 / src.w as f32).max(h as f32 / src.h as f32);
    let scaled = resize(src, (src.w as f32 * s).ceil() as i32, (src.h as f32 * s).ceil() as i32);
    let (ox, oy) = ((scaled.w - w) / 2, (scaled.h - h) / 2);
    let mut px = vec![255u8; (w * h) as usize];
    for y in 0..h.min(scaled.h) {
        for x in 0..w.min(scaled.w) {
            px[(y * w + x) as usize] = scaled.px[((y + oy) * scaled.w + x + ox) as usize];
        }
    }
    Gray { w, h, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_averages() {
        let g = Gray { w: 2, h: 2, px: vec![0, 255, 255, 0] };
        let r = resize(&g, 1, 1);
        assert_eq!(r.px, vec![127]);
        let f = fill(&Gray { w: 4, h: 2, px: vec![0; 8] }, 2, 2);
        assert_eq!((f.w, f.h), (2, 2));
    }
}
