//! Framebuffer output for i.MX EPDC Kindles (Wario platform) and a PNG simulator.
//!
//! The Kindle Basic 7th gen framebuffer: 600x800 visible, 8bpp grayscale, stride 608,
//! 0 = black. Refreshes are pushed with MXCFB_SEND_UPDATE using the Kindle (Wario-era)
//! struct layout, as FBInk does.

use crate::canvas::{Canvas, Rect};
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;

pub const WAVEFORM_DU: u32 = 1; // fast black/white (keyboard, small changes)
pub const WAVEFORM_GC16: u32 = 2; // full quality, flashes
pub const WAVEFORM_GL16: u32 = 5; // quality, less flashing
/// Greys darker than this become black in fast (DU) updates; a bit above 50% keeps strokes solid.
pub const DU_THRESHOLD: u8 = 168;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Update {
    Fast,
    Quality,
    Full,
}

pub trait Display {
    fn size(&self) -> (i32, i32);
    /// Copy `region` of the canvas to the screen and refresh it.
    fn present(&mut self, c: &Canvas, region: Rect, mode: Update);
    fn set_full_every(&mut self, _n: u32) {}
    fn snapshot(&mut self, _c: &Canvas, _name: &str) {}
}

#[repr(C)]
struct MxcfbUpdateData {
    top: u32,
    left: u32,
    width: u32,
    height: u32,
    waveform_mode: u32,
    update_mode: u32,
    update_marker: u32,
    hist_bw_waveform_mode: u32,
    hist_gray_waveform_mode: u32,
    temp: i32,
    flags: u32,
    alt_phys_addr: u32,
    alt_width: u32,
    alt_height: u32,
    alt_top: u32,
    alt_left: u32,
    alt_w: u32,
    alt_h: u32,
}

const fn ioc(dir: u32, ty: u8, nr: u32, size: usize) -> libc::c_ulong {
    ((dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr) as libc::c_ulong
}
const MXCFB_SEND_UPDATE: libc::c_ulong = ioc(1, b'F', 0x2E, std::mem::size_of::<MxcfbUpdateData>());
const MXCFB_WAIT_FOR_UPDATE_COMPLETE: libc::c_ulong = ioc(3, b'F', 0x2F, 8);
const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
const TEMP_USE_AMBIENT: i32 = 0x1000;

pub struct Eink {
    file: std::fs::File,
    map: *mut u8,
    map_len: usize,
    pub width: i32,
    pub height: i32,
    pub stride: usize,
    yoffset: usize,
    marker: u32,
    partials: u32,
    full_every: u32,
    pub id: String,
}

impl Eink {
    pub fn open(path: &str) -> io::Result<Eink> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let fd = file.as_raw_fd();
        let mut vinfo = [0u32; 40];
        let mut finfo = [0u8; 80];
        unsafe {
            if libc::ioctl(fd, FBIOGET_VSCREENINFO as _, vinfo.as_mut_ptr()) < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(fd, FBIOGET_FSCREENINFO as _, finfo.as_mut_ptr()) < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        let (xres, yres, xres_v, _yres_v, _xoff, yoff, bpp) =
            (vinfo[0], vinfo[1], vinfo[2], vinfo[3], vinfo[4], vinfo[5], vinfo[6]);
        // fb_fix_screeninfo (32-bit): id[16], smem_start(4), smem_len(4) @20, ..., line_length @44
        let id = String::from_utf8_lossy(&finfo[..16]).trim_end_matches('\0').to_string();
        let smem_len = u32::from_ne_bytes(finfo[20..24].try_into().unwrap()) as usize;
        let line_length = u32::from_ne_bytes(finfo[44..48].try_into().unwrap()) as usize;
        if bpp != 8 {
            return Err(io::Error::new(io::ErrorKind::Other, format!("unsupported {bpp}bpp framebuffer")));
        }
        let map = unsafe {
            libc::mmap(std::ptr::null_mut(), smem_len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        if map == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Eink {
            file,
            map: map as *mut u8,
            map_len: smem_len,
            width: xres as i32,
            height: yres as i32,
            stride: if line_length > 0 { line_length } else { xres_v as usize },
            yoffset: yoff as usize,
            marker: 0,
            partials: 0,
            full_every: 12,
            id,
        })
    }

    /// Copy a region to the framebuffer. With `binary`, greys are snapped to pure black/white
    /// first: the DU waveform can't show greys and renders anti-aliased edges unpredictably.
    fn blit(&mut self, c: &Canvas, r: Rect, binary: bool) {
        let w = (r.x1 - r.x0) as usize;
        let mut row = vec![0u8; w];
        for y in r.y0..r.y1 {
            let src = (y * c.w + r.x0) as usize;
            let dst = (self.yoffset + y as usize) * self.stride + r.x0 as usize;
            if dst + w > self.map_len {
                break;
            }
            let line = &c.buf[src..src + w];
            let data: &[u8] = if binary {
                for (o, &v) in row.iter_mut().zip(line) {
                    *o = if v < DU_THRESHOLD { 0 } else { 255 };
                }
                &row
            } else {
                line
            };
            unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), self.map.add(dst), w);
            }
        }
    }

    fn refresh(&mut self, r: Rect, waveform: u32, full: bool, wait: bool) {
        self.marker = self.marker % 65535 + 1;
        let data = MxcfbUpdateData {
            top: r.y0 as u32,
            left: r.x0 as u32,
            width: (r.x1 - r.x0) as u32,
            height: (r.y1 - r.y0) as u32,
            waveform_mode: waveform,
            update_mode: if full { 1 } else { 0 },
            update_marker: self.marker,
            hist_bw_waveform_mode: 0,
            hist_gray_waveform_mode: 0,
            temp: TEMP_USE_AMBIENT,
            flags: 0,
            alt_phys_addr: 0,
            alt_width: 0,
            alt_height: 0,
            alt_top: 0,
            alt_left: 0,
            alt_w: 0,
            alt_h: 0,
        };
        let fd = self.file.as_raw_fd();
        unsafe {
            libc::ioctl(fd, MXCFB_SEND_UPDATE as _, &data);
            if wait {
                let m: [u32; 2] = [self.marker, 0];
                libc::ioctl(fd, MXCFB_WAIT_FOR_UPDATE_COMPLETE as _, m.as_ptr());
            }
        }
    }
}

impl Display for Eink {
    fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    fn present(&mut self, c: &Canvas, region: Rect, mode: Update) {
        // EPDC prefers 8px-aligned regions
        let r = Rect::new(
            (region.x0 & !7).max(0),
            (region.y0 & !7).max(0),
            ((region.x1 + 7) & !7).min(self.width),
            ((region.y1 + 7) & !7).min(self.height),
        );
        if r.w() <= 0 || r.h() <= 0 {
            return;
        }
        self.blit(c, r, mode == Update::Fast);
        let mut full = mode == Update::Full;
        if mode == Update::Quality {
            self.partials += 1;
            if self.full_every > 0 && self.partials >= self.full_every {
                full = true;
            }
        }
        if full {
            self.partials = 0;
            self.refresh(r, WAVEFORM_GC16, true, true);
        } else if mode == Update::Fast {
            self.refresh(r, WAVEFORM_DU, false, false);
        } else {
            self.refresh(r, WAVEFORM_GL16, false, false);
        }
    }

    fn set_full_every(&mut self, n: u32) {
        self.full_every = n;
    }
}

impl Drop for Eink {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.map as *mut libc::c_void, self.map_len);
        }
    }
}

/// Simulator: keeps a copy of the "screen" and writes PNG snapshots.
pub struct Sim {
    pub screen: Canvas,
    pub out_dir: String,
    pub updates: Vec<(Rect, Update)>,
}

impl Sim {
    pub fn new(w: i32, h: i32, out_dir: &str) -> Sim {
        std::fs::create_dir_all(out_dir).ok();
        Sim { screen: Canvas::new(w, h), out_dir: out_dir.into(), updates: vec![] }
    }
}

impl Display for Sim {
    fn size(&self) -> (i32, i32) {
        (self.screen.w, self.screen.h)
    }

    fn present(&mut self, c: &Canvas, r: Rect, mode: Update) {
        for y in r.y0.max(0)..r.y1.min(c.h) {
            let s = (y * c.w + r.x0.max(0)) as usize;
            let e = (y * c.w + r.x1.min(c.w)) as usize;
            if mode == Update::Fast {
                // DU waveform: the panel can only show black or white; greys snap to one or the other
                for i in s..e {
                    self.screen.buf[i] = if c.buf[i] < DU_THRESHOLD { 0 } else { 255 };
                }
            } else {
                self.screen.buf[s..e].copy_from_slice(&c.buf[s..e]);
            }
        }
        self.updates.push((r, mode));
    }

    fn snapshot(&mut self, _c: &Canvas, name: &str) {
        let path = format!("{}/{}.png", self.out_dir, name);
        let file = std::fs::File::create(&path).expect("create png");
        let mut enc = png::Encoder::new(io::BufWriter::new(file), self.screen.w as u32, self.screen.h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&self.screen.buf).expect("png data");
        println!("saved {path}");
    }
}
