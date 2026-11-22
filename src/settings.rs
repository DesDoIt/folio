//! Wi-Fi, System info, Settings (SSH, time, display) and Power apps.

use crate::canvas::*;
use crate::input::Gesture;
use crate::sys::{self, Network, Sys};
use crate::ui::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn kv(cx: &mut Ctx, a: Rect, y: i32, k: &str, v: &str) {
    cx.text(a.x0 + 24, y, k, 20, DARK, Anchor::LT);
    cx.bold(a.x1 - 24, y, v, 20, BLACK, Anchor::RT);
}

/// Result slot for work done on a background thread.
type Slot<T> = Arc<Mutex<Option<T>>>;

// --- Wi-Fi ------------------------------------------------------------------------------
pub struct WifiApp {
    pager: Pager,
    networks: Vec<Network>,
    status: BTreeMap<String, String>,
    scan: Option<Slot<Result<(Vec<Network>, BTreeMap<String, String>), String>>>,
    connect: Option<(Slot<BTreeMap<String, String>>, Instant)>,
    sel: usize,
    ticks: u32,
}

const ID_TOGGLE: u32 = 1_000_000;
const ID_SCAN: u32 = 1_000_001;
const TAG_PASSWORD: u32 = 1;
const TAG_FORGET: u32 = 2;

impl WifiApp {
    pub fn new() -> WifiApp {
        WifiApp { pager: Pager::new(66), networks: vec![], status: BTreeMap::new(), scan: None, connect: None, sel: 0, ticks: 0 }
    }

    fn start_scan(&mut self, sys: &Sys) {
        if self.scan.is_some() {
            return;
        }
        let slot: Slot<_> = Arc::new(Mutex::new(None));
        let (s2, wifi) = (slot.clone(), sys.wifi.clone());
        std::thread::spawn(move || {
            let res = wifi.start().map(|()| {
                let nets = wifi.scan();
                (nets, wifi.status())
            });
            *s2.lock().unwrap() = Some(res);
        });
        self.scan = Some(slot);
    }

    fn start_connect(&mut self, sys: &Sys, ssid: String, pw: Option<String>) {
        let slot: Slot<_> = Arc::new(Mutex::new(None));
        let (s2, wifi) = (slot.clone(), sys.wifi.clone());
        std::thread::spawn(move || {
            wifi.connect(&ssid, pw.as_deref());
            let mut st = wifi.status();
            for _ in 0..20 {
                if st.contains_key("ip_address") {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
                st = wifi.status();
            }
            *s2.lock().unwrap() = Some(st);
        });
        self.connect = Some((slot, Instant::now()));
    }
}

impl App for WifiApp {
    fn title(&self) -> String {
        "Wi-Fi".into()
    }

    fn on_show(&mut self, sys: &mut Sys) {
        if sys.wifi.running() && self.networks.is_empty() {
            self.start_scan(sys);
        }
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        let on = sys.wifi.running();
        let scanning = self.scan.is_some();
        cx.button(Rect::new(a.x0 + 12, a.y0 + 10, a.x0 + 200, a.y0 + 62), if on { "Wi-Fi: ON" } else { "Wi-Fi: OFF" },
                  Some(Hit::App(ID_TOGGLE)), Btn { inverted: on, ..Btn::size(22) });
        cx.button(Rect::new(a.x1 - 180, a.y0 + 10, a.x1 - 12, a.y0 + 62), if scanning { "Scanning…" } else { "Scan" },
                  Some(Hit::App(ID_SCAN)), Btn { disabled: !on || scanning, ..Btn::size(22) });
        let y = a.y0 + 80;
        if !on {
            cx.text(a.cx(), y + 60, "Wi-Fi is off", 22, DARK, Anchor::MM);
            return;
        }
        let connected = self.status.get("wpa_state").map(String::as_str) == Some("COMPLETED");
        let ssid = if connected { self.status.get("ssid").cloned().unwrap_or_default() }
                   else if self.connect.is_some() { "connecting…".into() } else { "not connected".into() };
        kv(cx, a, y, "Network", &ssid);
        kv(cx, a, y + 32, "IP address", self.status.get("ip_address").map_or("—", |s| s.as_str()));
        cx.hline(a.x0 + 12, a.x1 - 12, y + 70, 1, BLACK);
        let nets = &self.networks;
        let status = &self.status;
        let empty = if scanning { "Scanning…" } else { "No networks found" };
        self.pager.render(cx, Rect::new(a.x0, y + 80, a.x1, a.y1), nets.len(), empty, |cx, r, i| {
            let n = &nets[i];
            let cur = connected && status.get("ssid") == Some(&n.ssid);
            let face = if cur { Face::Bold } else { Face::Sans };
            let name = cx.ellipsize(&n.ssid, face, 22, r.w() - 170);
            cx.c.text(cx.f, r.x0 + 24, r.y0 + 12, &name, face, 22, BLACK, Anchor::LT);
            let sub = format!("{}{}", if cur { "Connected · " } else { "" }, if n.secure { "Secured" } else { "Open" });
            cx.text(r.x0 + 24, r.y0 + 42, &sub, 15, DARK, Anchor::LT);
            let bars = if n.signal > -55 { 4 } else if n.signal > -65 { 3 } else if n.signal > -75 { 2 } else { 1 };
            for b in 0..4 {
                let h = 8 + b * 7;
                let bx = r.x1 - 80 + b * 13;
                cx.c.fill(Rect::new(bx, r.y0 + 46 - h, bx + 8, r.y0 + 46), if b < bars { BLACK } else { LIGHT });
            }
            cx.hit(r, Hit::App(i as u32), Some(Hit::App(0x8000_0000 | i as u32)), &n.ssid);
        });
    }

    fn busy(&self) -> bool {
        self.scan.is_some() || self.connect.is_some()
    }

    /// Keep the connection status live (DHCP can finish after the connect step returns).
    fn tick(&mut self, sys: &mut Sys) -> Cmd {
        self.ticks += 1;
        if self.ticks % 3 != 0 || !sys.wifi.running() || self.scan.is_some() || self.connect.is_some() {
            return Cmd::None;
        }
        let st = sys.wifi.status();
        let changed = st.get("wpa_state") != self.status.get("wpa_state") || st.get("ip_address") != self.status.get("ip_address")
            || st.get("ssid") != self.status.get("ssid");
        if changed {
            self.status = st;
            sys.invalidate_status();
            return Cmd::Redraw;
        }
        Cmd::None
    }

    fn poll(&mut self, _sys: &mut Sys) -> Cmd {
        let scanned = self.scan.as_ref().and_then(|s| s.lock().unwrap().take());
        if let Some(res) = scanned {
            self.scan = None;
            return match res {
                Ok((nets, st)) => {
                    self.networks = nets;
                    self.status = st;
                    Cmd::Redraw
                }
                Err(e) => Cmd::Batch(vec![Cmd::Redraw, Cmd::Dialog(Dialog::message("Wi-Fi didn't start",
                    &format!("{e}\n\nDetails are in kos/kos.log on the USB drive.")))]),
            };
        }
        let connected = self.connect.as_ref().and_then(|(s, _)| s.lock().unwrap().take());
        if let Some(st) = connected {
            let ok = st.contains_key("ip_address");
            self.status = st;
            self.connect = None;
            return Cmd::Batch(vec![Cmd::Redraw, Cmd::Toast(if ok { "Connected".into() } else { "Could not connect".into() })]);
        }
        Cmd::None
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        if self.pager.action(id) {
            return Cmd::Redraw;
        }
        match id {
            ID_TOGGLE => {
                if sys.wifi.running() {
                    sys.wifi.stop();
                    self.networks.clear();
                    self.status.clear();
                } else {
                    self.start_scan(sys); // starts wpa_supplicant first
                }
                sys.invalidate_status();
                Cmd::Redraw
            }
            ID_SCAN => {
                self.start_scan(sys);
                Cmd::Redraw
            }
            i if i & 0x8000_0000 != 0 => {
                self.sel = (i & 0x7fff_ffff) as usize;
                let ssid = self.networks.get(self.sel).map(|n| n.ssid.clone()).unwrap_or_default();
                Cmd::Dialog(Dialog::confirm(TAG_FORGET, "Forget network?", &ssid, "Forget"))
            }
            i => {
                let Some(n) = self.networks.get(i as usize).cloned() else { return Cmd::None };
                self.sel = i as usize;
                if n.secure {
                    Cmd::Dialog(Dialog::input(TAG_PASSWORD, &format!("Password for {}", n.ssid), "", "Connect")
                        .password().with_message("Tap the field to show or hide the password."))
                } else {
                    self.start_connect(sys, n.ssid.clone(), None);
                    Cmd::Toast(format!("Connecting to {}…", n.ssid))
                }
            }
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        let Some(n) = self.networks.get(self.sel).cloned() else { return Cmd::None };
        match tag {
            TAG_PASSWORD => {
                self.start_connect(sys, n.ssid.clone(), Some(text));
                Cmd::Toast(format!("Connecting to {}…", n.ssid))
            }
            TAG_FORGET => {
                sys.wifi.forget(&n.ssid);
                Cmd::Toast("Forgotten".into())
            }
            _ => Cmd::None,
        }
    }

    fn gesture(&mut self, g: &Gesture, _sys: &mut Sys) -> Cmd {
        if self.pager.swipe(g) { Cmd::Redraw } else { Cmd::None }
    }
}

// --- System -----------------------------------------------------------------------------
pub struct SystemApp;

impl App for SystemApp {
    fn title(&self) -> String {
        "System".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        let b = sys::battery(sys.sim);
        let (total, avail) = sys::meminfo(sys.sim);
        let (ut, uf) = sys::disk(if sys.sim { "/" } else { "/mnt/us" });
        let (rt, rf) = sys::disk("/");
        let rows: Vec<(&str, String)> = vec![
            ("Battery", format!("{}%  ({})", b.capacity.map_or("?".into(), |c| c.to_string()), b.status)),
            ("Voltage", format!("{:.2} V", b.voltage)),
            ("Battery temp", b.temp_c.map_or("?".into(), |t| format!("{t} °C"))),
            ("Charge cycles", b.cycles.map_or("?".into(), |c| c.to_string())),
            ("Memory available", format!("{avail} / {total} MB")),
            ("Storage free", format!("{:.2} / {:.2} GB", uf as f64 / 1024.0, ut as f64 / 1024.0)),
            ("System image free", format!("{rf} / {rt} MB")),
            ("Load average", sys::loadavg()),
            ("Uptime", sys::uptime()),
            ("Kernel", sys::kernel()),
            ("IP address", sys::ip_address(sys.sim).unwrap_or_else(|| "offline".into())),
            ("Kindle Linux", env!("CARGO_PKG_VERSION").into()),
        ];
        let mut y = a.y0 + 20;
        for (k, v) in rows {
            kv(cx, a, y, k, &v);
            y += 42;
            cx.hline(a.x0 + 16, a.x1 - 16, y - 10, 1, LIGHT);
        }
        let half = (a.w() - 44) / 2;
        cx.button(Rect::new(a.x0 + 16, a.y1 - 74, a.x0 + 16 + half, a.y1 - 16), "Battery history", Some(Hit::App(2)), Btn::size(20));
        cx.button(Rect::new(a.x1 - 16 - half, a.y1 - 74, a.x1 - 16, a.y1 - 16), "Refresh", Some(Hit::App(1)), Btn::size(20));
    }

    fn action(&mut self, id: u32, _long: bool, _sys: &mut Sys) -> Cmd {
        if id == 2 {
            return Cmd::Push(Box::new(crate::battery::BatteryApp));
        }
        Cmd::Redraw
    }
}

// --- Settings ---------------------------------------------------------------------------
const TIMEZONES: [&str; 8] = ["UTC", "Asia/Kolkata", "Europe/London", "Europe/Berlin", "America/New_York",
                              "America/Los_Angeles", "Asia/Tokyo", "Australia/Sydney"];
const REFRESH: [u32; 5] = [4, 8, 12, 20, 40];

pub struct SettingsApp {
    sync: Option<Slot<bool>>,
    page: usize,
    draft: Vec<String>,
}

const S_SSH: u32 = 1;
const S_PASSWORD: u32 = 2;
const S_SSH_AUTO: u32 = 3;
const S_TZ: u32 = 4;
const S_SYNC: u32 = 5;
const S_REFRESH: u32 = 6;
const S_WIFI_AUTO: u32 = 7;
const S_WEATHER: u32 = 8;
const S_SLEEPSCREEN: u32 = 10;
const S_AUTOSLEEP: u32 = 14;
const T_SLEEPSCREEN: u32 = 6;
const T_AUTOSLEEP: u32 = 7;
const AUTO_SLEEP: [u32; 6] = [2, 5, 10, 15, 30, 0];
const S_DASH_NOW: u32 = 11;
const S_PAGE: u32 = 12;
const S_SYSTEM: u32 = 13;
const S_WIFI_IDLE: u32 = 15;
const S_KOSYNC: u32 = 16;
const T_SYNC_SERVER: u32 = 9;
const T_SYNC_USER: u32 = 10;
const T_SYNC_PASS: u32 = 11;
const T_SYNC_REGISTER: u32 = 12;
const T_SYNC_SIGNOUT: u32 = 13;
const T_WIFI_IDLE: u32 = 8;
const WIFI_IDLE: [u32; 4] = [5, 15, 30, 0];
const T_WEATHER: u32 = 4;

const T_PASSWORD: u32 = 1;
const T_TZ: u32 = 2;
const T_REFRESH: u32 = 3;

impl SettingsApp {
    pub fn new() -> SettingsApp {
        SettingsApp { sync: None, page: 0, draft: vec![] }
    }
}

impl App for SettingsApp {
    fn title(&self) -> String {
        "Settings".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        let mut y = a.y0 + 16;
        let section = |cx: &mut Ctx, y: &mut i32, name: &str| {
            cx.bold(a.x0 + 20, *y, name, 16, DARK, Anchor::LT);
            *y += 30;
        };
        let item = |cx: &mut Ctx, y: &mut i32, id: u32, label: &str, value: &str, sub: Option<&str>| {
            let r = Rect::new(a.x0 + 12, *y, a.x1 - 12, *y + 60);
            cx.c.round_rect(r, 10, WHITE, 2, BLACK);
            cx.bold(r.x0 + 16, *y + if sub.is_some() { 10 } else { 18 }, label, 21, BLACK, Anchor::LT);
            if let Some(s) = sub {
                cx.text(r.x0 + 16, *y + 38, s, 14, DARK, Anchor::LT);
            }
            cx.text(r.x1 - 16, r.cy(), value, 19, BLACK, Anchor::RM);
            cx.hit(r, Hit::App(id), None, label);
            *y += 70;
        };
        let running = sys.ssh.running();
        let ip = sys::ip_address(sys.sim).unwrap_or_else(|| "no Wi-Fi".into());
        let ssh_sub = if running { format!("ssh root@{ip}") } else { "Port 22, password login".into() };
        if self.page == 0 {
        section(cx, &mut y, "SLEEP");
        let mode = crate::screensaver::mode_label(sys.config.get("sleep_screen"));
        let sub = if sys.config.get("sleep_screen") == "pictures" { Some("PNG/JPG files in the “screensavers” folder") } else { None };
        item(cx, &mut y, S_SLEEPSCREEN, "Sleep screen", mode, sub);
        let mins = sys.config.get("auto_sleep_minutes").parse::<u32>().unwrap_or(10);
        let auto = if mins == 0 { "never".to_string() } else { format!("{mins} min") };
        item(cx, &mut y, S_AUTOSLEEP, "Sleep after inactivity", &auto, None);
        let place = sys.config.get("weather_place").to_string();
        item(cx, &mut y, S_WEATHER, "Weather location", if place.is_empty() { "not set" } else { &place }, None);
        let upd = sys.dash.lock().unwrap().updated.map_or("never".to_string(), |u| sys::format_time(u, "%d %b %H:%M"));
        item(cx, &mut y, S_DASH_NOW, "Refresh weather", &upd, Some("Updates every 30 min while on Wi-Fi"));
        section(cx, &mut y, "REMOTE ACCESS");
        item(cx, &mut y, S_SSH, "SSH server", if running { "ON" } else { "OFF" }, Some(&ssh_sub));
        item(cx, &mut y, S_PASSWORD, "Root password", if sys.ssh.has_password() { "set" } else { "not set" }, None);
        item(cx, &mut y, S_SSH_AUTO, "Start SSH automatically", if sys.config.flag("ssh_autostart") { "yes" } else { "no" }, None);
        } else {
        section(cx, &mut y, "TIME");
        let tz = sys.config.get("timezone").to_string();
        item(cx, &mut y, S_TZ, "Time zone", &tz, None);
        let now = if self.sync.is_some() { "syncing…".to_string() } else { sys::clock("%d %b %H:%M") };
        item(cx, &mut y, S_SYNC, "Sync time now", &now, Some("Needs Wi-Fi (NTP)"));
        section(cx, &mut y, "DISPLAY & POWER");
        let fr = format!("{} updates", sys.config.get("full_refresh_every"));
        item(cx, &mut y, S_REFRESH, "Full refresh every", &fr, Some("Lower = less ghosting, more flashing"));
        item(cx, &mut y, S_WIFI_AUTO, "Connect Wi-Fi at start", if sys.config.flag("wifi_autostart") { "yes" } else { "no" }, None);
        let wi = sys.config.get("wifi_idle_off").parse::<u32>().unwrap_or(15);
        item(cx, &mut y, S_WIFI_IDLE, "Wi-Fi off when idle", &if wi == 0 { "never".to_string() } else { format!("after {wi} min") },
             Some("Saves battery; comes back when you tap"));
        section(cx, &mut y, "READING PROGRESS SYNC");
        let acct = sys.config.get("sync_user").to_string();
        item(cx, &mut y, S_KOSYNC, "KOReader sync account", if acct.is_empty() { "off" } else { &acct },
             Some("Continue books across devices"));
        section(cx, &mut y, "DEVICE");
        item(cx, &mut y, S_SYSTEM, "System info", "battery, memory…", None);
        }
        let label = if self.page == 0 { "More settings ▶" } else { "◀ Back to first page" };
        cx.button(Rect::new(a.x0 + 12, a.y1 - 66, a.x1 - 12, a.y1 - 12), label, Some(Hit::App(S_PAGE)), Btn::size(20));
    }

    fn busy(&self) -> bool {
        self.sync.is_some()
    }

    fn poll(&mut self, sys: &mut Sys) -> Cmd {
        let done = self.sync.as_ref().and_then(|s| s.lock().unwrap().take());
        if let Some(ok) = done {
            self.sync = None;
            sys.invalidate_status();
            return Cmd::Batch(vec![Cmd::Redraw, Cmd::Toast(if ok { "Time synced".into() } else { "Sync failed".into() })]);
        }
        Cmd::None
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        let flip = |sys: &mut Sys, k: &str| {
            let v = if sys.config.flag(k) { "no" } else { "yes" };
            sys.config.set(k, v);
            Cmd::Redraw
        };
        match id {
            S_SSH => {
                if sys.ssh.running() {
                    sys.ssh.stop();
                    sys.invalidate_status();
                    Cmd::Toast("SSH stopped".into())
                } else if !sys.ssh.has_password() {
                    Cmd::Dialog(Dialog::message("Set a password first", "SSH needs a root password. Tap 'Root password'."))
                } else {
                    sys.ssh.start();
                    sys.invalidate_status();
                    Cmd::Toast("SSH started on port 22".into())
                }
            }
            S_PASSWORD => Cmd::Dialog(Dialog::input(T_PASSWORD, "New root password", "", "Save").password()),
            S_SYSTEM => Cmd::Push(Box::new(SystemApp)),
            S_KOSYNC => {
                if sys.config.get("sync_user").is_empty() {
                    self.draft.clear();
                    let server = Some(sys.config.get("sync_server").to_string()).filter(|s| !s.is_empty())
                        .unwrap_or_else(|| crate::sync::DEFAULT_SERVER.into());
                    Cmd::Dialog(Dialog::input(T_SYNC_SERVER, "Sync server", &server, "Next")
                        .with_message("KOReader's public server works; use the same account as in KOReader"))
                } else {
                    Cmd::Dialog(Dialog::confirm(T_SYNC_SIGNOUT, "Sign out of sync?", sys.config.get("sync_user"), "Sign out"))
                }
            }
            S_WIFI_IDLE => Cmd::Dialog(Dialog::menu(T_WIFI_IDLE, "Wi-Fi off when idle",
                WIFI_IDLE.iter().map(|m| if *m == 0 { "Never".to_string() } else { format!("After {m} minutes") }).collect())),
            S_PAGE => {
                self.page = 1 - self.page;
                Cmd::Full
            }
            S_SSH_AUTO => flip(sys, "ssh_autostart"),
            S_WEATHER => Cmd::Dialog(Dialog::input(T_WEATHER, "Weather location", sys.config.get("weather_place"), "Find")
                .with_message("City name, e.g. Pune or Berlin")),
            S_SLEEPSCREEN => Cmd::Dialog(Dialog::menu(T_SLEEPSCREEN, "Sleep screen",
                crate::screensaver::MODES.iter().map(|m| m.1.to_string()).collect())),
            S_AUTOSLEEP => Cmd::Dialog(Dialog::menu(T_AUTOSLEEP, "Sleep after inactivity",
                AUTO_SLEEP.iter().map(|m| if *m == 0 { "Never".to_string() } else { format!("{m} minutes") }).collect())),
            S_DASH_NOW => {
                sys.dash_refresh_now = true;
                Cmd::Toast("Refreshing…".into())
            }
            S_WIFI_AUTO => flip(sys, "wifi_autostart"),
            S_TZ => Cmd::Dialog(Dialog::menu(T_TZ, "Time zone", TIMEZONES.iter().map(|s| s.to_string()).collect())),
            S_REFRESH => Cmd::Dialog(Dialog::menu(T_REFRESH, "Full refresh every…",
                                                  REFRESH.iter().map(|n| format!("{n} updates")).collect())),
            S_SYNC => {
                if self.sync.is_none() && !sys.sim {
                    let slot: Slot<bool> = Arc::new(Mutex::new(None));
                    let s2 = slot.clone();
                    std::thread::spawn(move || {
                        let ok = sys::ok(&["ntpd", "-n", "-q", "-p", "pool.ntp.org"], false);
                        if ok {
                            sys::run(&["hwclock", "-w", "-u"], false);
                        }
                        *s2.lock().unwrap() = Some(ok);
                    });
                    self.sync = Some(slot);
                }
                Cmd::Toast("Syncing time…".into())
            }
            _ => Cmd::None,
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, text: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match tag {
            T_PASSWORD => {
                if text.chars().count() < 6 {
                    return Cmd::Dialog(Dialog::message("Too short", "Use at least 6 characters."));
                }
                match sys.ssh.set_password(&text) {
                    Ok(()) => Cmd::Toast("Password updated".into()),
                    Err(e) => Cmd::Dialog(Dialog::message("Failed", &e)),
                }
            }
            T_WEATHER => match crate::dashboard::geocode(&text) {
                Ok((lat, lon, label)) => {
                    sys.config.set("weather_lat", &format!("{lat:.4}"));
                    sys.config.set("weather_lon", &format!("{lon:.4}"));
                    sys.config.set("weather_place", &label);
                    sys.dash_refresh_now = true;
                    Cmd::Toast(format!("Weather for {label}"))
                }
                Err(e) => Cmd::Dialog(Dialog::message("Couldn't find that place", &e)),
            },
            T_SLEEPSCREEN => {
                if let Some(m) = crate::screensaver::MODES.get(choice) {
                    sys.config.set("sleep_screen", m.0);
                    if m.0 == "pictures" {
                        let _ = std::fs::create_dir_all(crate::screensaver::pictures_dir(sys));
                    }
                }
                Cmd::Redraw
            }
            T_SYNC_SERVER => {
                self.draft = vec![text.trim().trim_end_matches('/').to_string()];
                Cmd::Dialog(Dialog::input(T_SYNC_USER, "Username", "", "Next"))
            }
            T_SYNC_USER => {
                self.draft.push(text.trim().to_string());
                Cmd::Dialog(Dialog::input(T_SYNC_PASS, "Password", "", "Sign in").password())
            }
            T_SYNC_PASS | T_SYNC_REGISTER => {
                if tag == T_SYNC_PASS {
                    self.draft.push(crate::sync::hex(&crate::sync::md5(text.as_bytes())));
                }
                if self.draft.len() < 3 || self.draft[1].is_empty() {
                    return Cmd::None;
                }
                let acct = crate::sync::Account { server: self.draft[0].clone(), user: self.draft[1].clone(), key: self.draft[2].clone() };
                let result = if tag == T_SYNC_REGISTER { acct.register().map(|_| true) } else { acct.authorize() };
                match result {
                    Ok(true) => {
                        sys.config.set("sync_server", &acct.server);
                        sys.config.set("sync_user", &acct.user);
                        sys.config.set("sync_key", &acct.key);
                        if sys.config.get("sync_device_id").is_empty() {
                            let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
                            sys.config.set("sync_device_id", &crate::sync::hex(&crate::sync::md5(seed.as_bytes())));
                        }
                        Cmd::Toast("Sync is on".into())
                    }
                    Ok(false) => Cmd::Dialog(Dialog::confirm(T_SYNC_REGISTER, "Account not found",
                        "Unknown user or wrong password. Create a new account with these details?", "Create account")),
                    Err(e) => Cmd::Dialog(Dialog::message("Sync", &e)),
                }
            }
            T_SYNC_SIGNOUT => {
                sys.config.set("sync_user", "");
                sys.config.set("sync_key", "");
                Cmd::Toast("Signed out".into())
            }
            T_WIFI_IDLE => {
                if let Some(m) = WIFI_IDLE.get(choice) {
                    sys.config.set("wifi_idle_off", &m.to_string());
                }
                Cmd::Redraw
            }
            T_AUTOSLEEP => {
                if let Some(m) = AUTO_SLEEP.get(choice) {
                    sys.config.set("auto_sleep_minutes", &m.to_string());
                }
                Cmd::Redraw
            }
            T_TZ => {
                if let Some(tz) = TIMEZONES.get(choice) {
                    sys.config.set("timezone", tz);
                    sys::set_timezone(tz, sys.sim);
                }
                Cmd::Redraw
            }
            T_REFRESH => {
                if let Some(n) = REFRESH.get(choice) {
                    sys.config.set("full_refresh_every", &n.to_string());
                    sys.full_every_changed = Some(*n);
                }
                Cmd::Redraw
            }
            _ => Cmd::None,
        }
    }
}

// --- Power ------------------------------------------------------------------------------
pub struct PowerApp;

const P_SLEEP: u32 = 1;
const P_EXIT: u32 = 2;
const P_RESTART: u32 = 3;
const P_REBOOT: u32 = 4;
const P_OFF: u32 = 5;

impl App for PowerApp {
    fn title(&self) -> String {
        "Power".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, _sys: &mut Sys) {
        let items: [(u32, &str, &str); 5] = [
            (P_SLEEP, "Sleep", "Screen shows sleep image; power button wakes"),
            (P_EXIT, "Exit to stock Kindle", "Amazon's interface: USB drive mode, store books"),
            (P_RESTART, "Restart interface", "Reload the Kindle Linux GUI"),
            (P_REBOOT, "Reboot", "Restart the device"),
            (P_OFF, "Power off", "Shut down completely"),
        ];
        let mut y = a.y0 + 20;
        for (id, label, sub) in items {
            cx.button(Rect::new(a.x0 + 16, y, a.x1 - 16, y + 100), label, Some(Hit::App(id)),
                      Btn { size: 26, bold: true, sub: Some(sub), ..Default::default() });
            y += 116;
        }
    }

    fn action(&mut self, id: u32, _long: bool, sys: &mut Sys) -> Cmd {
        match id {
            P_SLEEP => {
                sys.sleep_requested = true;
                Cmd::None
            }
            P_EXIT => {
                sys.request("exit", "");
                Cmd::Quit(0)
            }
            P_RESTART => Cmd::Quit(3),
            P_REBOOT => Cmd::Dialog(Dialog::confirm(P_REBOOT, "Reboot?", "Restart the Kindle now?", "Reboot")),
            P_OFF => Cmd::Dialog(Dialog::confirm(P_OFF, "Power off?", "Shut the Kindle down?", "Power off")),
            _ => Cmd::None,
        }
    }

    fn dialog(&mut self, tag: u32, choice: usize, _t: String, sys: &mut Sys) -> Cmd {
        if choice == CANCEL {
            return Cmd::None;
        }
        match tag {
            P_REBOOT => {
                sys.request("reboot", "");
                Cmd::Quit(20)
            }
            P_OFF => {
                sys.request("poweroff", "");
                Cmd::Quit(21)
            }
            _ => Cmd::None,
        }
    }
}
