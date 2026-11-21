//! Kindle Linux GUI.
//!
//! Exit codes understood by the launcher (host/kos-launch.sh):
//!   0 exit to stock Kindle   3 restart GUI   10 open KOReader (path in /mnt/us/kos/request)
//!   11 open Amazon's browser (URL in /mnt/us/kos/request)   20 reboot   21 power off   1 crashed

mod apps;
mod bootmenu;
mod canvas;
mod dashboard;
mod epub;
mod fb;
mod games;
mod getbooks;
mod highlights;
mod input;
mod json;
mod news;
mod reader;
mod settings;
mod sketch;
mod sys;
mod term;
mod ui;
mod upload;
mod web;
mod wiki;

use canvas::*;
use fb::{Display, Update};
use input::{Gesture, Touch};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};
use sys::Sys;
use ui::Ui;

fn log(msg: &str) {
    let line = format!("{} [gui] {msg}", sys::clock("%Y-%m-%d %H:%M:%S"));
    // the launcher redirects our stderr into kos/kos.log
    eprintln!("{line}");
}

fn sleep_screen(disp: &mut dyn Display, fonts: &mut Fonts, battery: Option<u32>) {
    let (w, h) = disp.size();
    let mut c = Canvas::new(w, h);
    c.outline(Rect::new(40, 40, w - 40, h - 40), 4, BLACK);
    c.text(fonts, w / 2, h / 2 - 60, "Sleeping", Face::Bold, 54, BLACK, Anchor::MM);
    c.text(fonts, w / 2, h / 2 + 10, "Press the power button to wake", Face::Sans, 22, DARK, Anchor::MM);
    let b = battery.map_or("?".into(), |b| b.to_string());
    c.text(fonts, w / 2, h - 90, &format!("Kindle Linux · battery {b}%"), Face::Sans, 18, DARK, Anchor::MM);
    disp.present(&c, Rect::new(0, 0, w, h), Update::Full);
}

fn crash_screen(disp: &mut dyn Display, msg: &str) {
    let (w, h) = disp.size();
    let mut fonts = Fonts::load();
    let mut c = Canvas::new(w, h);
    c.text(&mut fonts, 20, 40, "Kindle Linux crashed", Face::Bold, 30, BLACK, Anchor::LT);
    let mut y = 100;
    for line in msg.lines().take(20) {
        for chunk in line.chars().collect::<Vec<_>>().chunks(58) {
            c.text(&mut fonts, 20, y, &chunk.iter().collect::<String>(), Face::Mono, 14, BLACK, Anchor::LT);
            y += 20;
        }
    }
    c.text(&mut fonts, 20, h - 60, "Returning to the stock Kindle in 10 s…", Face::Sans, 20, BLACK, Anchor::LT);
    disp.present(&c, Rect::new(0, 0, w, h), Update::Full);
}

/// Reads powerd events from `lipc-wait-event` without blocking.
struct Watcher {
    child: std::process::Child,
    buf: String,
}

impl Watcher {
    fn fd(&self) -> i32 {
        self.child.stdout.as_ref().unwrap().as_raw_fd()
    }
    fn lines(&mut self) -> Vec<String> {
        let mut tmp = [0u8; 512];
        if let Some(out) = self.child.stdout.as_mut() {
            if let Ok(n) = out.read(&mut tmp) {
                self.buf.push_str(&String::from_utf8_lossy(&tmp[..n]));
            }
        }
        let mut out = vec![];
        while let Some(i) = self.buf.find('\n') {
            out.push(self.buf[..i].trim().to_string());
            self.buf.drain(..=i);
        }
        out
    }
}

struct Main {
    ui: Ui,
    sys: Sys,
    sleeping: bool,
    /// Sleep was requested (menu or power button); if powerd hasn't reacted in time we suspend ourselves.
    sleep_pending: Option<Instant>,
    /// Set by the fallback-suspend thread when the device has woken up again.
    woke: std::sync::Arc<std::sync::atomic::AtomicBool>,
    self_suspended: bool,
    ignore_power_until: Option<Instant>,
    last_dash: Option<Instant>,
    dash_busy: std::sync::Arc<std::sync::Mutex<bool>>,
}

impl Main {
    fn new(sim: bool, w: i32, h: i32) -> Main {
        let sys = Sys::new(sim);
        sys::set_timezone(&sys.config.get("timezone").to_string(), sim);
        let mut m = Main { ui: Ui::new(w, h), sys, sleeping: false, sleep_pending: None,
                           woke: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), self_suspended: false,
                           ignore_power_until: None, last_dash: None,
                           dash_busy: std::sync::Arc::new(std::sync::Mutex::new(false)) };
        m.ui.push(Box::new(apps::Home::new()), &mut m.sys);
        m
    }

    fn startup(&mut self) {
        self.sys.power.amazon_screensaver(false);
        if self.sys.config.flag("wifi_autostart") {
            let w = self.sys.wifi.clone();
            std::thread::spawn(move || {
                if let Err(e) = w.start() {
                    sys::log(&format!("wifi autostart failed: {e}"));
                }
            });
        }
        if self.sys.config.flag("ssh_autostart") && self.sys.ssh.has_password() {
            self.sys.ssh.start();
        }
    }

    fn enter_sleep(&mut self, disp: &mut dyn Display) {
        if !self.sleeping {
            self.sleeping = true;
            log("going to screensaver");
            let b = sys::battery(self.sys.sim).capacity;
            let dash_on = self.sys.config.get("sleep_screen") != "simple" && dashboard::configured_location(&self.sys).is_some();
            if dash_on {
                let d = self.sys.dash.lock().unwrap().clone();
                dashboard::draw_sleep(disp, &mut self.ui.fonts, &d, b);
            } else {
                sleep_screen(disp, &mut self.ui.fonts, b);
            }
        }
    }

    /// Kick off a background dashboard refresh every 30 minutes (or when asked).
    fn maybe_refresh_dashboard(&mut self) {
        let due = self.sys.dash_refresh_now || self.last_dash.map_or(true, |t| t.elapsed() > Duration::from_secs(1800));
        if !due || self.dash_busy.lock().map_or(true, |b| *b) {
            return;
        }
        let loc = dashboard::configured_location(&self.sys);
        self.sys.dash_refresh_now = false;
        self.last_dash = Some(Instant::now());
        let Some(loc) = loc else { return };
        let (slot, busy) = (self.sys.dash.clone(), self.dash_busy.clone());
        *busy.lock().unwrap() = true;
        std::thread::spawn(move || {
            let d = dashboard::refresh(loc);
            if let Some(e) = &d.error {
                sys::log(&format!("dashboard: {e}"));
            }
            let mut cur = slot.lock().unwrap();
            // keep the last good weather if this refresh failed (e.g. Wi-Fi was down)
            let keep_weather = d.weather.is_none() && cur.weather.is_some();
            let old_weather = cur.weather.take();
            *cur = d;
            if keep_weather {
                cur.weather = old_weather;
            }
            *busy.lock().unwrap() = false;
        });
    }

    fn leave_sleep(&mut self) {
        if self.sleeping {
            self.sleeping = false;
            log("out of screensaver");
            self.sys.invalidate_status();
            self.ui.request(Update::Full);
        }
    }

    fn after_input(&mut self, disp: &mut dyn Display) {
        if let Some(n) = self.sys.full_every_changed.take() {
            disp.set_full_every(n);
        }
        if self.sys.sleep_requested {
            self.sys.sleep_requested = false;
            self.sys.power.sleep();
            self.sleep_pending = Some(Instant::now());
            if self.sys.sim {
                self.enter_sleep(disp);
            }
        }
    }

    /// powerd didn't start the screensaver: draw our sleep screen and suspend via the kernel.
    fn fallback_sleep(&mut self, disp: &mut dyn Display) {
        log("sleep: powerd didn't react, suspending directly");
        self.enter_sleep(disp);
        self.self_suspended = true;
        let woke = self.woke.clone();
        woke.store(false, std::sync::atomic::Ordering::SeqCst);
        // give the e-ink refresh time to finish before the display controller is powered down
        std::thread::sleep(Duration::from_millis(1500));
        let r = self.sys.power.suspend_now();
        log(&format!("sleep: resumed ({})", match r { Ok(()) => "ok".to_string(), Err(e) => e }));
        woke.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn run(&mut self, disp: &mut dyn Display, mut touch: Option<Touch>) -> i32 {
        let mut power_key = if self.sys.sim { None } else {
            input::find_power_key().and_then(|p| input::PowerKey::open(&p).ok())
        };
        log(&format!("power key: {}", if power_key.is_some() { "found" } else { "not found" }));
        disp.set_full_every(self.sys.config.get("full_refresh_every").parse().unwrap_or(12));
        self.startup();
        let mut watcher = self.sys.power.watcher().map(|child| {
            let fd = child.stdout.as_ref().unwrap().as_raw_fd();
            unsafe {
                let fl = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
            }
            Watcher { child, buf: String::new() }
        });
        let mut last_tick = Instant::now() - Duration::from_secs(2);
        let mut last_min = sys::clock("%H:%M");
        let mut first_frame = !self.sys.sim;
        while self.ui.quit.is_none() {
            if !self.sleeping {
                self.ui.render(disp, &mut self.sys);
                if first_frame {
                    first_frame = false;
                    mark_boot_ok();
                }
            }
            // poll set: touch, powerd watcher, top app's fd
            let mut fds: Vec<libc::pollfd> = vec![];
            let touch_fd = touch.as_ref().map(|t| t.fd());
            let watch_fd = watcher.as_ref().map(|w| w.fd());
            let app_fd = self.ui.top().fd();
            let key_fd = power_key.as_ref().map(|k| k.fd());
            for fd in [touch_fd, watch_fd, app_fd, key_fd].into_iter().flatten() {
                fds.push(libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
            }
            let fast = touch.as_ref().map_or(false, |t| t.down) || self.ui.top().busy() || self.ui.has_toast()
                || self.sleep_pending.is_some()
                || self.ui.cleanup_pending();
            let timeout = if fast { 100 } else { 1000 };
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
            for p in &fds {
                if p.revents == 0 {
                    continue;
                }
                if Some(p.fd) == touch_fd {
                    let raw_mode = self.ui.wants_raw();
                    let t = touch.as_mut().unwrap();
                    t.raw = if raw_mode { Some(t.raw.take().unwrap_or_default()) } else { None };
                    let gestures = t.read();
                    let raw_events = t.raw.as_mut().map(std::mem::take).unwrap_or_default();
                    if !self.sleeping {
                        self.sys.power.poke();
                        let raw_top = if raw_mode { self.ui.raw_top() } else { 0 };
                        for ev in raw_events {
                            let y = match ev { ui::RawTouch::Down(_, y) | ui::RawTouch::Move(_, y) | ui::RawTouch::Up(_, y) => y };
                            if y >= raw_top || matches!(ev, ui::RawTouch::Up(..)) {
                                self.ui.raw_touch(ev, &mut self.sys);
                            }
                        }
                        for g in gestures {
                            // in drawing apps only taps above the canvas (header, toolbar) are gestures
                            if !raw_mode || g.y < raw_top {
                                self.ui.gesture(&g, &mut self.sys);
                            }
                        }
                    }
                } else if Some(p.fd) == watch_fd {
                    for line in watcher.as_mut().unwrap().lines() {
                        if line.contains("goingToScreenSaver") {
                            self.sleep_pending = None;
                            self.enter_sleep(disp);
                        } else if line.contains("outOfScreenSaver") {
                            self.ignore_power_until = Some(Instant::now() + Duration::from_secs(2));
                            self.leave_sleep();
                        } else if !line.is_empty() {
                            self.sys.invalidate_status();
                            self.ui.request(Update::Quality);
                        }
                    }
                } else if Some(p.fd) == key_fd {
                    let n = power_key.as_mut().unwrap().presses();
                    let ignore = self.ignore_power_until.map_or(false, |t| Instant::now() < t);
                    if n > 0 && !ignore && !self.sleeping {
                        // powerd normally handles the button itself; only start the fallback timer
                        log("power key pressed");
                        self.sleep_pending = Some(Instant::now());
                    }
                } else if Some(p.fd) == app_fd {
                    let c = self.ui.top().on_fd(&mut self.sys);
                    self.ui.apply(c, &mut self.sys);
                }
            }
            if let Some(t) = touch.as_mut() {
                if let Some(g) = t.pending_long() {
                    if !self.sleeping {
                        self.ui.gesture(&g, &mut self.sys);
                    }
                }
            }
            let c = self.ui.top().poll(&mut self.sys);
            self.ui.apply(c, &mut self.sys);
            self.ui.expire_toast();
            self.ui.idle_cleanup();
            // sleep fallback: powerd didn't start the screensaver within 4 s
            if self.sleep_pending.map_or(false, |t| t.elapsed() > Duration::from_secs(4)) && !self.sleeping {
                self.sleep_pending = None;
                self.fallback_sleep(disp);
            }
            if self.self_suspended && self.woke.load(std::sync::atomic::Ordering::SeqCst) {
                self.self_suspended = false;
                self.ignore_power_until = Some(Instant::now() + Duration::from_secs(2));
                if let Some(k) = power_key.as_mut() {
                    k.presses(); // drop the wake-up press
                }
                self.leave_sleep();
            }
            self.after_input(disp);
            if last_tick.elapsed() >= Duration::from_secs(1) && !self.sleeping {
                last_tick = Instant::now();
                self.maybe_refresh_dashboard();
                let c = self.ui.top().tick(&mut self.sys);
                self.ui.apply(c, &mut self.sys);
                let m = sys::clock("%H:%M");
                if m != last_min {
                    last_min = m;
                    self.ui.request(Update::Quality); // header clock
                }
            }
        }
        let code = self.ui.quit.unwrap();
        log(&format!("exiting with code {code}"));
        if matches!(code, 0 | 20 | 21) {
            self.sys.ssh.stop();
            self.sys.wifi.stop();
        }
        if let Some(mut w) = watcher {
            let _ = w.child.kill();
            let _ = w.child.wait();
        }
        if matches!(code, 0 | 11 | 20 | 21) {
            self.sys.power.amazon_screensaver(true);
        }
        code
    }
}

fn boot_menu_main(secs: u64) -> i32 {
    let Ok(mut disp) = fb::Eink::open("/dev/fb0") else { return 0 };
    let touch = Touch::open(&input::find_touch(), disp.width, disp.height, None).ok();
    if bootmenu::run(&mut disp, touch, secs) { 0 } else { 1 }
}

/// Tell the boot script the GUI came up fine (clears the boot-failure marker).
fn mark_boot_ok() {
    let _ = std::fs::remove_file("/mnt/us/kos/boot-pending");
}

fn device_main() -> i32 {
    let mut disp = match fb::Eink::open("/dev/fb0") {
        Ok(d) => d,
        Err(e) => {
            log(&format!("cannot open framebuffer: {e}"));
            return 1;
        }
    };
    log(&format!("display {}x{} stride={} id={}", disp.width, disp.height, disp.stride, disp.id));
    let tpath = input::find_touch();
    let debug = std::path::Path::new("/mnt/us/kos/debug-touch").exists().then(|| "/mnt/us/kos/touch.log".to_string());
    let touch = match Touch::open(&tpath, disp.width, disp.height, debug) {
        Ok(t) => {
            log(&format!("touch {tpath} max={:?}", t.max()));
            Some(t)
        }
        Err(e) => {
            log(&format!("touch {tpath} failed: {e}"));
            None
        }
    };
    let (w, h) = (disp.width, disp.height);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut m = Main::new(false, w, h);
        m.run(&mut disp, touch)
    }));
    match result {
        Ok(code) => code,
        Err(e) => {
            let msg = e.downcast_ref::<String>().cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown panic".into());
            log(&format!("CRASH: {msg}"));
            crash_screen(&mut disp, &msg);
            std::thread::sleep(Duration::from_secs(10));
            1
        }
    }
}

// --- Simulator -------------------------------------------------------------------------
struct SimRun {
    m: Main,
    disp: fb::Sim,
    n: u32,
}

impl SimRun {
    fn render(&mut self) {
        self.m.ui.render(&mut self.disp, &mut self.m.sys);
    }
    fn shot(&mut self, name: &str) {
        self.render();
        self.n += 1;
        let name = format!("{:02}-{name}", self.n);
        self.m.ui.snapshot(&mut self.disp, &name);
    }
    fn tap(&mut self, label: &str) {
        self.render();
        let r = self.m.ui.find_label(label)
            .unwrap_or_else(|| panic!("no button {label:?}; have {:?}", self.m.ui.labels()));
        self.m.ui.gesture(&Gesture::tap(r.cx(), r.cy()), &mut self.m.sys);
        self.m.after_input(&mut self.disp);
        self.render();
    }
    /// Tap the n-th hit box with this exact label (e.g. the 3rd "link").
    fn tap_nth(&mut self, label: &str, n: usize) -> bool {
        self.render();
        let r = self.m.ui.hits.iter().filter(|h| h.label == label).nth(n).map(|h| h.r);
        match r {
            Some(r) => {
                self.m.ui.gesture(&Gesture::tap(r.cx(), r.cy()), &mut self.m.sys);
                self.render();
                true
            }
            None => false,
        }
    }

    fn typ(&mut self, s: &str) {
        for ch in s.chars() {
            match ch {
                ' ' => self.tap("space"),
                '\n' => self.tap("⏎"),
                c if c.is_ascii_uppercase() => {
                    self.tap("⇧");
                    self.tap(&c.to_string());
                }
                c if c.is_ascii_alphanumeric() || c == ',' || c == '.' => self.tap(&c.to_string()),
                c => {
                    self.tap("?123");
                    self.tap(&c.to_string());
                    self.tap("abc");
                }
            }
        }
    }
    /// Close an error/info dialog if one is showing (external services can be down).
    fn dismiss(&mut self) {
        if self.m.ui.find_label("OK").is_some() {
            println!("note: dismissed a dialog (service unavailable?)");
            self.tap("OK");
        }
    }

    /// Pump background jobs until the top app is no longer busy (or `secs` pass).
    fn wait_idle(&mut self, secs: u64) {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            let c = self.m.ui.top().poll(&mut self.m.sys);
            self.m.ui.apply(c, &mut self.m.sys);
            if !self.m.ui.top().busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.render();
    }

    fn settle(&mut self, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            let c = self.m.ui.top().on_fd(&mut self.m.sys);
            self.m.ui.apply(c, &mut self.m.sys);
            let c = self.m.ui.top().poll(&mut self.m.sys);
            self.m.ui.apply(c, &mut self.m.sys);
            std::thread::sleep(Duration::from_millis(30));
        }
        self.m.ui.request(Update::Quality);
    }
}

fn bootmenu_preview(d: &mut fb::Sim, _f: &mut Fonts) {
    // Render the boot menu once (no touch: it would count down, so just draw the first frame).
    let mut fonts = Fonts::load();
    let _ = &mut fonts;
    struct Once<'a>(&'a mut fb::Sim, bool);
    impl Display for Once<'_> {
        fn size(&self) -> (i32, i32) { self.0.size() }
        fn present(&mut self, c: &Canvas, r: Rect, m: Update) {
            if !self.1 { self.0.present(c, r, m); self.0.snapshot(c, "17-boot-menu"); self.1 = true; }
        }
    }
    let mut once = Once(d, false);
    bootmenu::run(&mut once, None, 1);
}

fn make_userstore(us: &str) {
    let docs = format!("{us}/documents");
    let _ = std::fs::create_dir_all(&docs);
    for name in ["Three Men in a Boat_B004UJL1KK.kfx", "Pride and Prejudice.epub", "The Time Machine.epub",
                 "Linux Pocket Guide.pdf", "Meditations - Marcus Aurelius.azw3", "Notes on e-ink.txt"] {
        let _ = std::fs::write(format!("{docs}/{name}"), "sample ".repeat(200));
    }
    let _ = std::fs::copy("tests/fixtures/sample.epub", format!("{docs}/A Sample Book.epub"));
    let _ = std::fs::remove_file(format!("{us}/kos/reading.txt"));
    let _ = std::fs::create_dir_all(format!("{us}/notes"));
    let _ = std::fs::write(format!("{us}/notes/todo.txt"), "Kindle Linux todo\n- test wifi\n- try ssh\n- read more\n");
}

fn sim_main(out: &str) -> i32 {
    let us = sys::userstore(true);
    make_userstore(&us);
    let mut s = SimRun { m: Main::new(true, 600, 800), disp: fb::Sim::new(600, 800, out), n: 0 };
    s.shot("home");
    s.tap("Books");
    s.shot("books");
    s.tap("‹");
    s.tap("Files");
    s.shot("files");
    s.tap("documents");
    s.shot("files-docs");
    s.tap("⌂");
    s.tap("Notes");
    s.shot("notes");
    s.tap("Kindle Linux todo");
    s.shot("editor");
    s.typ("Hello Kindle");
    s.shot("editor-typed");
    s.tap("⌂");
    s.tap("Wi-Fi");
    s.settle(300);
    s.shot("wifi");
    s.tap("HomeNetwork");
    s.typ("secret");
    s.shot("wifi-password");
    s.tap("Cancel");
    s.tap("⌂");
    s.tap("Settings");
    s.tap("More settings ▶");
    s.tap("System info");
    s.shot("system");
    s.tap("‹");
    s.tap("◀ Back to first page");
    s.shot("settings");
    s.tap("More settings ▶");
    s.tap("Time zone");
    s.shot("settings-tz");
    s.tap("Asia/Kolkata");
    s.tap("‹");
    s.tap("Power");
    s.shot("power");
    s.tap("‹");
    s.tap("Terminal");
    s.settle(500);
    s.typ("uname -m; echo hello from kindle-linux\n");
    s.settle(800);
    s.shot("terminal");
    s.tap("⌂");
    s.tap("Terminal"); // resumes the same shell
    s.settle(200);
    s.shot("terminal-resumed");
    s.tap("⌂");
    // built-in reader
    s.tap("Books");
    let t0 = Instant::now();
    s.tap("A Sample Book");
    println!("reader: opened + laid out chapter in {} ms", t0.elapsed().as_millis());
    s.shot("reader-page1");
    s.tap("next page");
    s.shot("reader-page2");
    for _ in 0..6 {
        s.tap("next page");
    }
    s.shot("reader-later");
    s.tap("menu");
    s.shot("reader-menu");
    s.tap("Chapters");
    s.shot("reader-chapters");
    s.tap("Chapter Two: The Road North");
    s.shot("reader-chapter2");
    s.tap("menu");
    s.tap("Font size");
    s.tap("28 px");
    s.shot("reader-font28");
    s.tap("menu");
    s.tap("Back to library");
    s.tap("⌂");
    // Get Books: live network (popular Gutenberg list, Standard Ebooks, a search and a real download)
    s.tap("Get Books");
    s.wait_idle(60);
    s.shot("getbooks-gutenberg");
    s.dismiss();
    s.tap("Standard Ebooks");
    s.wait_idle(60);
    s.shot("getbooks-standard");
    s.dismiss();
    s.tap("Search");
    s.typ("time machine");
    s.tap("Search");
    s.wait_idle(60);
    s.shot("getbooks-search");
    s.dismiss();
    if s.m.ui.find_label("The Time Machine").is_some() {
        s.tap("The Time Machine");
        s.tap("Download");
        s.wait_idle(120);
        s.shot("getbooks-downloaded");
        if s.m.ui.find_label("Read now").is_some() {
            s.tap("Read now");
            s.shot("getbooks-reading");
            s.tap("menu");
            s.tap("Back to library");
        }
    }
    s.tap("⌂");
    // Web browser: start page, live search, follow a result, reading view
    s.tap("Web");
    s.shot("web-start");
    s.tap("address");
    s.typ("e ink display");
    s.tap("Go");
    s.wait_idle(60);
    s.shot("web-search");
    if s.m.ui.find_label("Use other engine").is_some() {
        s.tap("Use other engine");
        s.wait_idle(60);
        s.shot("web-search-marginalia");
    }
    // DuckDuckGo Lite: skip its own nav links, open the first real result
    let mut opened = false;
    for i in 0..12 {
        let label_rect = s.m.ui.hits.iter().filter(|h| h.label == "link").nth(i).map(|h| h.r);
        if label_rect.map_or(false, |r| r.y0 > 200) {
            opened = s.tap_nth("link", i);
            break;
        }
    }
    if opened {
        s.wait_idle(60);
        s.shot("web-page");
        if s.m.ui.find_label("Use other engine").is_some() {
            s.tap("Cancel"); // the result was behind a bot check: the browser explained it
        }
        s.tap("☰");
        s.tap("Reading view: OFF");
        s.shot("web-reading-view");
    }
    // Wikipedia pages full of non-ASCII text (regression test for the UTF-8 slicing crash)
    for (i, page) in ["en.m.wikipedia.org/wiki/Tokyo", "ja.m.wikipedia.org/wiki/%E6%9D%B1%E4%BA%AC%E9%83%BD",
                      "fr.m.wikipedia.org/wiki/Caf%C3%A9"].iter().enumerate() {
        if s.m.ui.find_label("Use other engine").is_some() {
            s.tap("Cancel");
        }
        s.tap("address");
        s.typ(page);
        s.tap("Go");
        s.wait_idle(60);
        s.shot(&format!("web-wikipedia-{i}"));
        s.tap("Next ▶");
        s.tap("Next ▶");
    }
    s.tap("⌂");
    // --- fun apps ---
    // Highlights: long-press a word in the reader, then "Highlight sentence"
    let sample = format!("{}/documents/A Sample Book.epub", s.m.sys.us);
    let rd = reader::Reader::open(&sample, &s.m.sys).expect("sample book");
    s.m.ui.push(Box::new(rd), &mut s.m.sys);
    s.render();
    let w = s.m.ui.hits.iter().find(|h| h.label == "next page").map(|h| h.r).unwrap();
    let _ = w;
    for (x, y) in [(300, 250), (200, 420), (150, 520), (320, 610), (250, 330)] {
        s.m.ui.gesture(&Gesture { kind: input::Kind::Long, x, y, x2: x, y2: y }, &mut s.m.sys);
        s.render();
        if s.m.ui.find_label("Highlight sentence").is_some() {
            break;
        }
        if s.m.ui.find_label("Close menu").is_some() {
            s.tap("Close menu");
        }
    }
    s.shot("reader-word-menu");
    if s.m.ui.find_label("Highlight sentence").is_some() {
        s.tap("Highlight sentence");
    }
    s.tap("menu");
    s.tap("Back to library");
    s.tap("Highlights");
    s.shot("highlights");
    s.tap("⌂");
    s.tap("Stats");
    s.shot("stats");
    s.tap("⌂");
    // News: a local sample feed (original text, so screenshots carry no third-party articles)
    let feed = format!("file://{}/tests/fixtures/sample-feed.xml", std::env::current_dir().unwrap().display());
    let _ = std::fs::create_dir_all(format!("{}/kos", s.m.sys.us));
    let _ = std::fs::write(format!("{}/kos/feeds.txt", s.m.sys.us), format!("{feed}\nhttps://hnrss.org/frontpage\n"));
    s.tap("News");
    s.shot("news-feeds");
    s.tap(feed.split("://").nth(1).unwrap());
    s.wait_idle(60);
    s.dismiss();
    s.shot("news-items");
    let first = s.m.ui.hits.iter().find(|h| h.r.y0 > 150 && !h.label.is_empty() && h.label.len() > 12).map(|h| h.label.clone());
    if let Some(t) = first {
        s.tap(&t);
        s.wait_idle(60);
        s.dismiss();
        s.shot("news-article");
        if s.m.ui.find_label("menu").is_some() {
            s.tap("menu");
            s.tap("Back to library");
        }
    }
    s.tap("⌂");
    // Wikipedia: search, open the first result
    s.tap("Wikipedia");
    s.tap("Search");
    s.typ("electronic paper");
    s.tap("Search");
    s.wait_idle(60);
    s.dismiss();
    s.shot("wiki-results");
    if s.m.ui.find_label("Electronic paper").is_some() {
        s.tap("Electronic paper");
        s.wait_idle(60);
        s.dismiss();
        s.shot("wiki-article");
        s.tap("menu");
        s.tap("Chapters");
        s.shot("wiki-sections");
        s.tap("‹");
        s.tap("menu");
        s.tap("Back to library");
    }
    s.tap("⌂");
    // Games
    s.tap("Games");
    s.shot("games");
    s.tap("Sudoku");
    s.shot("sudoku");
    s.tap("‹");
    s.tap("Chess");
    s.tap("sq52"); // e2
    s.tap("sq36"); // e4
    s.wait_idle(60);
    s.shot("chess");
    s.tap("⌂");
    // Sketch: a few strokes, save, view the PNG
    s.tap("Sketch");
    for (x0, y0, x1, y1) in [(100, 300, 500, 300), (100, 300, 300, 600), (500, 300, 300, 600)] {
        s.m.ui.raw_touch(ui::RawTouch::Down(x0, y0), &mut s.m.sys);
        for k in 1..=20 {
            s.m.ui.raw_touch(ui::RawTouch::Move(x0 + (x1 - x0) * k / 20, y0 + (y1 - y0) * k / 20), &mut s.m.sys);
        }
        s.m.ui.raw_touch(ui::RawTouch::Up(x1, y1), &mut s.m.sys);
    }
    s.shot("sketch");
    s.tap("Save");
    s.tap("⌂");
    let sk_dir = format!("{}/sketches", s.m.sys.us);
    if let Some(f) = std::fs::read_dir(&sk_dir).ok().and_then(|mut rd| rd.next()).and_then(|e| e.ok()) {
        let path = f.path().to_string_lossy().into_owned();
        s.m.ui.push(Box::new(sketch::ImageView::new(&path)), &mut s.m.sys);
        s.shot("image-view");
        s.tap("⌂");
    }
    // Send to Kindle: upload the sample EPUB like a phone browser would
    s.tap("Send to");
    std::thread::sleep(Duration::from_millis(300));
    let up = std::process::Command::new("curl").args(["-s", "-o", "/dev/null", "-w", "%{http_code}",
        "-F", "file=@tests/fixtures/sample.epub;filename=Uploaded Sample.epub", "http://127.0.0.1:8080/upload"]).output();
    println!("upload: {:?}", up.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()));
    std::thread::sleep(Duration::from_millis(1200));
    let c = s.m.ui.top().tick(&mut s.m.sys);
    s.m.ui.apply(c, &mut s.m.sys);
    s.shot("send-to-kindle");
    println!("uploaded file exists: {}", std::path::Path::new(&format!("{}/documents/Uploaded Sample.epub", s.m.sys.us)).exists());
    s.tap("⌂");
    // Weather dashboard: live forecast
    match dashboard::geocode("London") {
        Ok((lat, lon, label)) => {
            s.m.sys.config.set("weather_lat", &format!("{lat}"));
            s.m.sys.config.set("weather_lon", &format!("{lon}"));
            s.m.sys.config.set("weather_place", &label);
        }
        Err(e) => println!("geocode failed: {e}"),
    }
    s.m.sys.config.set("sleep_screen", "dashboard");
    let d = dashboard::refresh(dashboard::configured_location(&s.m.sys).expect("location"));
    println!("dashboard: weather={} error={:?}", d.weather.is_some(), d.error);
    *s.m.sys.dash.lock().unwrap() = d;
    s.m.ui.request(Update::Full);
    s.shot("home-weather");
    s.tap("Settings");
    s.shot("settings-dashboard");
    s.tap("‹");
    s.m.sleeping = false;
    s.m.enter_sleep(&mut s.disp);
    s.n += 1;
    let name = format!("{:02}-dashboard-sleep", s.n);
    s.disp.snapshot(&Canvas::new(1, 1), &name);
    s.m.leave_sleep();
    s.tap("Power");
    s.m.sys.config.set("sleep_screen", "simple");
    s.tap("Sleep");
    s.n += 1;
    {
        let mut d = fb::Sim::new(600, 800, out);
        let mut f = Fonts::load();
        bootmenu_preview(&mut d, &mut f);
    }
    let name = format!("{:02}-sleep", s.n);
    s.disp.snapshot(&Canvas::new(1, 1), &name);
    println!("updates: {} ({} full)", s.disp.updates.len(),
             s.disp.updates.iter().filter(|u| u.1 == Update::Full).count());
    0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = if args.get(1).map(String::as_str) == Some("--sim") {
        sim_main(args.get(2).map(String::as_str).unwrap_or("sim-out"))
    } else if args.get(1).map(String::as_str) == Some("--boot-menu") {
        let secs = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
        boot_menu_main(secs)
    } else {
        signal_setup();
        device_main()
    };
    std::process::exit(code);
}

fn signal_setup() {
    // record where a crash happened (file:line) in kos.log, not just the message
    std::panic::set_hook(Box::new(|info| {
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let msg = info.payload().downcast_ref::<String>().cloned()
            .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
        log(&format!("PANIC at {loc}: {msg}"));
    }));
    unsafe {
        // children (udhcpc, dropbear helpers) are reaped by waitpid/Child; ignore SIGPIPE from closed ptys
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}
