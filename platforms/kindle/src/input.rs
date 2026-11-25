//! evdev touch input turned into gestures (tap, long press, swipes).
//! Kindle Basic 7th gen: /dev/input/event1 = zforce2 (MT protocol B), event0 = power key.

use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::time::Instant;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_MT_SLOT: u16 = 47;
const ABS_MT_POSITION_X: u16 = 53;
const ABS_MT_POSITION_Y: u16 = 54;
const ABS_MT_TRACKING_ID: u16 = 57;
const BTN_TOUCH: u16 = 330;

/// struct input_event on 32-bit ARM: timeval (2 x i32), u16 type, u16 code, i32 value = 16 bytes.
/// On 64-bit (simulator host) timeval is 2 x i64 = 24 bytes.
const EVENT_SIZE: usize = std::mem::size_of::<libc::timeval>() + 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tap,
    Long,
    SwipeLeft,
    SwipeRight,
    SwipeUp,
    SwipeDown,
}

#[derive(Clone, Copy, Debug)]
pub struct Gesture {
    pub kind: Kind,
    pub x: i32,
    pub y: i32,
    /// End point of the gesture (where the finger lifted); kept for swipe-distance use.
    #[allow(dead_code)]
    pub x2: i32,
    #[allow(dead_code)]
    pub y2: i32,
}

impl Gesture {
    pub fn tap(x: i32, y: i32) -> Gesture {
        Gesture { kind: Kind::Tap, x, y, x2: x, y2: y }
    }
}

fn eviocgname(len: usize) -> libc::c_ulong {
    ((2u32 << 30) | ((len as u32) << 16) | ((b'E' as u32) << 8) | 0x06) as libc::c_ulong
}

fn eviocgabs(code: u16) -> libc::c_ulong {
    ((2u32 << 30) | (24u32 << 16) | ((b'E' as u32) << 8) | (0x40 + code as u32)) as libc::c_ulong
}

pub fn device_name(path: &str) -> String {
    let Ok(f) = File::open(path) else { return String::new() };
    let mut buf = [0u8; 256];
    let r = unsafe { libc::ioctl(f.as_raw_fd(), eviocgname(256) as _, buf.as_mut_ptr()) };
    if r < 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Find the touchscreen by name; fall back to the Kindle default.
pub fn find_touch() -> String {
    for i in 0..8 {
        let p = format!("/dev/input/event{i}");
        let n = device_name(&p).to_lowercase();
        if ["zforce", "touch", "cyttsp", "elan"].iter().any(|k| n.contains(k)) {
            return p;
        }
    }
    "/dev/input/event1".into()
}

pub struct Touch {
    file: File,
    sw: i32,
    sh: i32,
    maxx: i32,
    maxy: i32,
    pub swap_xy: bool,
    pub mirror_x: bool,
    pub mirror_y: bool,
    slot: i32,
    pub down: bool,
    raw_xy: (Option<i32>, Option<i32>),
    start: Option<(i32, i32)>,
    last: Option<(i32, i32)>,
    t0: Instant,
    long_sent: bool,
    log: Option<String>,
    /// When set, finger positions are also queued here (for drawing apps).
    pub raw: Option<Vec<crate::ui::RawTouch>>,
    last_raw: Option<(i32, i32)>,
}

pub const TAP_SLOP: i32 = 28;
pub const LONG_PRESS_MS: u128 = 650;

impl Touch {
    pub fn open(path: &str, sw: i32, sh: i32, log: Option<String>) -> std::io::Result<Touch> {
        let file = std::fs::OpenOptions::new().read(true).custom_flags_nonblock().open(path)?;
        let mut t = Touch {
            file,
            sw,
            sh,
            maxx: sw - 1,
            maxy: sh - 1,
            swap_xy: false,
            mirror_x: false,
            mirror_y: false,
            slot: 0,
            down: false,
            raw_xy: (None, None),
            start: None,
            last: None,
            t0: Instant::now(),
            long_sent: false,
            log,
            raw: None,
            last_raw: None,
        };
        t.maxx = t.absmax(ABS_MT_POSITION_X).or(t.absmax(ABS_X)).unwrap_or(sw - 1);
        t.maxy = t.absmax(ABS_MT_POSITION_Y).or(t.absmax(ABS_Y)).unwrap_or(sh - 1);
        Ok(t)
    }

    pub fn max(&self) -> (i32, i32) {
        (self.maxx, self.maxy)
    }

    fn absmax(&self, code: u16) -> Option<i32> {
        let mut info = [0i32; 6];
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), eviocgabs(code) as _, info.as_mut_ptr()) };
        if r < 0 || info[2] <= 0 {
            None
        } else {
            Some(info[2])
        }
    }

    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    fn map(&self, rx: i32, ry: i32) -> (i32, i32) {
        let (rx, ry, mx, my) = if self.swap_xy { (ry, rx, self.maxy, self.maxx) } else { (rx, ry, self.maxx, self.maxy) };
        let mut x = (rx as i64 * (self.sw - 1) as i64 / mx.max(1) as i64) as i32;
        let mut y = (ry as i64 * (self.sh - 1) as i64 / my.max(1) as i64) as i32;
        if self.mirror_x {
            x = self.sw - 1 - x;
        }
        if self.mirror_y {
            y = self.sh - 1 - y;
        }
        (x.clamp(0, self.sw - 1), y.clamp(0, self.sh - 1))
    }

    /// A long press is reported while the finger is still down.
    pub fn pending_long(&mut self) -> Option<Gesture> {
        if self.down && !self.long_sent {
            if let (Some(s), Some(l)) = (self.start, self.last) {
                if self.t0.elapsed().as_millis() >= LONG_PRESS_MS && (l.0 - s.0).abs() + (l.1 - s.1).abs() <= TAP_SLOP {
                    self.long_sent = true;
                    return Some(Gesture { kind: Kind::Long, x: s.0, y: s.1, x2: s.0, y2: s.1 });
                }
            }
        }
        None
    }

    pub fn read(&mut self) -> Vec<Gesture> {
        let mut out = Vec::new();
        let mut buf = [0u8; EVENT_SIZE * 64];
        loop {
            let n = match self.file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            let tv = std::mem::size_of::<libc::timeval>();
            for ev in buf[..n].chunks_exact(EVENT_SIZE) {
                let typ = u16::from_ne_bytes([ev[tv], ev[tv + 1]]);
                let code = u16::from_ne_bytes([ev[tv + 2], ev[tv + 3]]);
                let value = i32::from_ne_bytes([ev[tv + 4], ev[tv + 5], ev[tv + 6], ev[tv + 7]]);
                if let Some(g) = self.event(typ, code, value) {
                    out.push(g);
                }
            }
        }
        out
    }

    fn event(&mut self, typ: u16, code: u16, value: i32) -> Option<Gesture> {
        match typ {
            EV_ABS => match code {
                ABS_MT_SLOT => self.slot = value,
                _ if self.slot != 0 => {}
                ABS_MT_POSITION_X | ABS_X => self.raw_xy.0 = Some(value),
                ABS_MT_POSITION_Y | ABS_Y => self.raw_xy.1 = Some(value),
                ABS_MT_TRACKING_ID => {
                    if value >= 0 {
                        self.begin()
                    } else {
                        return self.end();
                    }
                }
                _ => {}
            },
            EV_KEY if code == BTN_TOUCH => {
                if value != 0 {
                    self.begin()
                } else {
                    return self.end();
                }
            }
            EV_SYN if self.down => {
                if let (Some(rx), Some(ry)) = self.raw_xy {
                    let p = self.map(rx, ry);
                    if let Some(q) = self.raw.as_mut() {
                        if self.last_raw.is_none() {
                            q.push(crate::ui::RawTouch::Down(p.0, p.1));
                        } else if self.last_raw != Some(p) {
                            q.push(crate::ui::RawTouch::Move(p.0, p.1));
                        }
                        self.last_raw = Some(p);
                    }
                    if self.start.is_none() {
                        self.start = Some(p);
                        self.logline(&format!("down raw=({rx},{ry}) mapped={p:?}"));
                    }
                    self.last = Some(p);
                }
            }
            _ => {}
        }
        None
    }

    fn begin(&mut self) {
        if !self.down {
            self.down = true;
            self.start = None;
            self.last = None;
            self.t0 = Instant::now();
            self.long_sent = false;
        }
    }

    fn end(&mut self) -> Option<Gesture> {
        if !self.down {
            return None;
        }
        self.down = false;
        if let (Some(q), Some(p)) = (self.raw.as_mut(), self.last_raw.take()) {
            q.push(crate::ui::RawTouch::Up(p.0, p.1));
        }
        if self.start.is_none() {
            let (Some(rx), Some(ry)) = self.raw_xy else { return None };
            let p = self.map(rx, ry);
            self.start = Some(p);
            self.last = Some(p);
        }
        if self.long_sent {
            return None;
        }
        let (s, l) = (self.start.unwrap(), self.last.unwrap());
        let (dx, dy) = (l.0 - s.0, l.1 - s.1);
        let kind = if dx.abs() <= TAP_SLOP && dy.abs() <= TAP_SLOP {
            if self.t0.elapsed().as_millis() >= LONG_PRESS_MS { Kind::Long } else { Kind::Tap }
        } else if dx.abs() > dy.abs() {
            if dx > 0 { Kind::SwipeRight } else { Kind::SwipeLeft }
        } else if dy > 0 {
            Kind::SwipeDown
        } else {
            Kind::SwipeUp
        };
        Some(Gesture { kind, x: s.0, y: s.1, x2: l.0, y2: l.1 })
    }

    fn logline(&self, msg: &str) {
        if let Some(p) = &self.log {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "{msg}");
            }
        }
    }
}

trait NonBlock {
    fn custom_flags_nonblock(&mut self) -> &mut Self;
}

impl NonBlock for std::fs::OpenOptions {
    fn custom_flags_nonblock(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.custom_flags(libc::O_NONBLOCK)
    }
}

/// The power button (max77696-onkey on the Kindle Basic 7th gen).
pub struct PowerKey {
    file: File,
}

pub fn find_power_key() -> Option<String> {
    (0..8).map(|i| format!("/dev/input/event{i}"))
        .find(|p| { let n = device_name(p).to_lowercase(); n.contains("onkey") || n.contains("power") })
}

impl PowerKey {
    pub fn open(path: &str) -> std::io::Result<PowerKey> {
        let file = std::fs::OpenOptions::new().read(true).custom_flags_nonblock().open(path)?;
        Ok(PowerKey { file })
    }

    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    /// Number of key-down presses since the last call.
    pub fn presses(&mut self) -> usize {
        let mut buf = [0u8; EVENT_SIZE * 16];
        let mut n = 0;
        let tv = std::mem::size_of::<libc::timeval>();
        while let Ok(len) = self.file.read(&mut buf) {
            if len == 0 {
                break;
            }
            for ev in buf[..len].chunks_exact(EVENT_SIZE) {
                let typ = u16::from_ne_bytes([ev[tv], ev[tv + 1]]);
                let code = u16::from_ne_bytes([ev[tv + 2], ev[tv + 3]]);
                let value = i32::from_ne_bytes([ev[tv + 4], ev[tv + 5], ev[tv + 6], ev[tv + 7]]);
                if typ == EV_KEY && code == 116 && value == 1 {
                    n += 1;
                }
            }
        }
        n
    }
}
