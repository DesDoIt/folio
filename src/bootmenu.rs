//! Boot menu shown at power-on: "Kindle Linux" (default after a countdown) or "Stock Kindle".
//! Runs directly on the Kindle host (static binary), before anything is mounted.
//! Exit code: 0 = Kindle Linux, 1 = stock Kindle.

use crate::canvas::*;
use crate::fb::{Display, Update};
use crate::input::{Kind, Touch};
use std::time::{Duration, Instant};

const LINUX: Rect = Rect::new(60, 330, 540, 470);
const STOCK: Rect = Rect::new(60, 510, 540, 650);

fn draw(disp: &mut dyn Display, fonts: &mut Fonts, secs: u64, chosen: Option<bool>, full: bool) {
    let (w, h) = disp.size();
    let mut c = Canvas::new(w, h);
    c.text(fonts, w / 2, 150, "Kindle Linux", Face::Bold, 48, BLACK, Anchor::MM);
    let msg = match chosen {
        Some(true) => "Starting Kindle Linux…".to_string(),
        Some(false) => "Starting the stock Kindle…".to_string(),
        None => format!("Starting Kindle Linux in {secs} s"),
    };
    c.text(fonts, w / 2, 230, &msg, Face::Sans, 26, DARK, Anchor::MM);
    c.round_rect(LINUX, 18, BLACK, 0, BLACK);
    c.text(fonts, LINUX.cx(), LINUX.cy() - 14, "Kindle Linux", Face::Bold, 34, WHITE, Anchor::MM);
    c.text(fonts, LINUX.cx(), LINUX.cy() + 30, "Alpine + touch GUI", Face::Sans, 18, WHITE, Anchor::MM);
    c.round_rect(STOCK, 18, WHITE, 3, BLACK);
    c.text(fonts, STOCK.cx(), STOCK.cy() - 14, "Stock Kindle", Face::Bold, 34, BLACK, Anchor::MM);
    c.text(fonts, STOCK.cx(), STOCK.cy() + 30, "Amazon interface, USB drive, store books", Face::Sans, 18, DARK, Anchor::MM);
    c.text(fonts, w / 2, h - 70, "Create kos/disable on the USB drive to always boot the stock Kindle",
           Face::Sans, 15, DARK, Anchor::MM);
    let mode = if full { Update::Full } else { Update::Fast };
    disp.present(&c, Rect::new(0, 0, w, h), mode);
}

/// Returns true for Kindle Linux, false for the stock Kindle.
pub fn run(disp: &mut dyn Display, mut touch: Option<Touch>, timeout_s: u64) -> bool {
    let mut fonts = Fonts::load();
    let start = Instant::now();
    let mut shown = u64::MAX;
    let mut first = true;
    loop {
        let elapsed = start.elapsed().as_secs();
        if elapsed >= timeout_s {
            draw(disp, &mut fonts, 0, Some(true), false);
            return true;
        }
        let left = timeout_s - elapsed;
        if left != shown {
            shown = left;
            draw(disp, &mut fonts, left, None, first);
            first = false;
        }
        if let Some(t) = touch.as_mut() {
            let mut pfd = libc::pollfd { fd: t.fd(), events: libc::POLLIN, revents: 0 };
            unsafe { libc::poll(&mut pfd, 1, 200) };
            for g in t.read() {
                if matches!(g.kind, Kind::Tap | Kind::Long) {
                    if LINUX.contains(g.x, g.y) {
                        draw(disp, &mut fonts, 0, Some(true), false);
                        return true;
                    }
                    if STOCK.contains(g.x, g.y) {
                        draw(disp, &mut fonts, 0, Some(false), false);
                        return false;
                    }
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
