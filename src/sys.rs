//! System services: battery, memory, storage, clock, Wi-Fi (own wpa_supplicant), SSH (dropbear),
//! power (via Amazon's powerd, which owns safe suspend on this kernel) and config.
//!
//! Runs inside the Alpine chroot; the Kindle root is bind-mounted read-only at /kindle so
//! lipc/iptables can be reached with `chroot /kindle ...`.

use std::collections::BTreeMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const HOST: &str = "/kindle";
pub const STATE: &str = "/var/lib/kos";
pub const IFACE: &str = "wlan0";
const WPA_CTRL: &str = "/run/wpa_supplicant";
const BATTERY: &str = "/sys/class/power_supply/max77696-battery";

pub fn userstore(sim: bool) -> String {
    if sim { std::env::var("KOS_SIM_US").unwrap_or_else(|_| "/tmp/kindle-sim-us".into()) } else { "/mnt/us".into() }
}

/// Append a line to the shared log on the Kindle drive (kos/kos.log).
pub fn log(msg: &str) {
    // the launcher redirects our stderr into kos/kos.log
    eprintln!("{} [gui] {msg}", clock("%Y-%m-%d %H:%M:%S"));
}

pub fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Run a command, returning stdout+stderr ("" on failure). `host` runs it in the Kindle root.
pub fn run(args: &[&str], host: bool) -> String {
    let mut cmd = if host {
        let mut c = Command::new("chroot");
        c.arg(HOST).args(args);
        c
    } else {
        let mut c = Command::new(args[0]);
        c.args(&args[1..]);
        c
    };
    match cmd.stdin(Stdio::null()).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        }
        Err(_) => String::new(),
    }
}

pub fn ok(args: &[&str], host: bool) -> bool {
    let mut cmd = if host {
        let mut c = Command::new("chroot");
        c.arg(HOST).args(args);
        c
    } else {
        let mut c = Command::new(args[0]);
        c.args(&args[1..]);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_or(false, |s| s.success())
}

pub fn clock(fmt: &str) -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0u8; 128];
        let cfmt = std::ffi::CString::new(fmt).unwrap();
        let n = libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cfmt.as_ptr(), &tm);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

// libc marks `time_t` deprecated on 32-bit musl (it will become 64-bit); the cast is still correct here.
#[allow(deprecated)]
pub fn format_time(t: i64, fmt: &str) -> String {
    unsafe {
        let tt = t as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&tt, &mut tm);
        let mut buf = [0u8; 128];
        let cfmt = std::ffi::CString::new(fmt).unwrap();
        let n = libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cfmt.as_ptr(), &tm);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

extern "C" {
    fn tzset();
}

pub fn set_timezone(tz: &str, sim: bool) {
    let src = format!("/usr/share/zoneinfo/{tz}");
    if !sim && std::path::Path::new(&src).exists() {
        let _ = std::fs::remove_file("/etc/localtime");
        let _ = std::os::unix::fs::symlink(&src, "/etc/localtime");
    }
    std::env::set_var("TZ", tz);
    unsafe { tzset() };
}

#[derive(Default, Clone)]
pub struct Battery {
    pub capacity: Option<u32>,
    pub status: String,
    pub voltage: f32,
    pub temp_c: Option<i32>,
    pub cycles: Option<u32>,
}

impl Battery {
    pub fn charging(&self) -> bool {
        self.status == "Charging" || self.status == "Full"
    }
}

pub fn battery(sim: bool) -> Battery {
    if sim {
        return Battery { capacity: Some(87), status: "Discharging".into(), voltage: 3.98, temp_c: Some(24),
                         cycles: Some(178) };
    }
    let num = |f: &str| read(&format!("{BATTERY}/{f}")).and_then(|s| s.parse::<i64>().ok());
    Battery {
        capacity: num("capacity").map(|v| v as u32),
        status: read(&format!("{BATTERY}/status")).unwrap_or_else(|| "Unknown".into()),
        voltage: num("voltage_now").unwrap_or(0) as f32 / 1000.0,
        // the max77696 gauge on Kindles reports Fahrenheit
        temp_c: num("temp").map(|f| ((f - 32) * 5 / 9) as i32),
        cycles: num("cycle_count").map(|v| v as u32),
    }
}

/// (total MB, available MB)
pub fn meminfo(sim: bool) -> (u64, u64) {
    if sim {
        return (249, 196);
    }
    let mut m = BTreeMap::new();
    for line in read("/proc/meminfo").unwrap_or_default().lines() {
        let mut it = line.split_whitespace();
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            m.insert(k.trim_end_matches(':').to_string(), v.parse::<u64>().unwrap_or(0));
        }
    }
    let total = m.get("MemTotal").copied().unwrap_or(0);
    // Kernel 3.0 has no MemAvailable: free + buffers + page cache is what programs can actually get.
    let avail = m.get("MemAvailable").copied().unwrap_or(
        m.get("MemFree").copied().unwrap_or(0) + m.get("Buffers").copied().unwrap_or(0)
            + m.get("Cached").copied().unwrap_or(0));
    (total / 1024, avail / 1024)
}

/// (total MB, free MB)
pub fn disk(path: &str) -> (u64, u64) {
    let c = std::ffi::CString::new(path).unwrap();
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return (0, 0);
    }
    let bs = s.f_frsize as u64;
    (s.f_blocks as u64 * bs >> 20, s.f_bavail as u64 * bs >> 20)
}

pub fn uptime() -> String {
    let s: f64 = read("/proc/uptime").and_then(|u| u.split_whitespace().next().map(String::from))
        .and_then(|v| v.parse().ok()).unwrap_or(0.0);
    format!("{}h {}m", (s / 3600.0) as u64, (s % 3600.0 / 60.0) as u64)
}

pub fn loadavg() -> String {
    read("/proc/loadavg").map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

pub fn kernel() -> String {
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    unsafe { libc::uname(&mut u) };
    let r: Vec<u8> = u.release.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&r).into_owned()
}

pub fn ip_address(sim: bool) -> Option<String> {
    if sim {
        return Some("192.168.1.50".into());
    }
    let out = run(&["ip", "-4", "addr", "show", IFACE], false);
    out.split_whitespace().skip_while(|w| *w != "inet").nth(1).map(|a| a.split('/').next().unwrap().to_string())
}

// --- Config -----------------------------------------------------------------------
pub struct Config {
    path: String,
    /// Copy on the USB drive (without secrets), used if the main file is ever missing.
    backup: String,
    pub values: BTreeMap<String, String>,
}

/// Settings that are secrets: kept only in the main config, never in the USB-drive copy.
const SECRET_KEYS: &[&str] = &["sync_key"];

impl Config {
    pub fn load(sim: bool) -> Config {
        let path = if sim { "/tmp/kos-sim-config".to_string() } else { format!("{STATE}/config") };
        let backup = format!("{}/kos/settings.txt", userstore(sim));
        let mut values = BTreeMap::new();
        for (k, v) in [("full_refresh_every", "12"), ("timezone", "UTC"), ("ssh_autostart", "no"),
                       ("wifi_autostart", "yes")] {
            values.insert(k.to_string(), v.to_string());
        }
        let main = read(&path).unwrap_or_default();
        let text = if main.trim().is_empty() {
            let b = read(&backup).unwrap_or_default();
            if !b.trim().is_empty() {
                log("config: main settings missing, restored from kos/settings.txt");
            }
            b
        } else {
            main
        };
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                values.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        Config { path, backup, values }
    }
    pub fn get(&self, k: &str) -> &str {
        self.values.get(k).map(String::as_str).unwrap_or("")
    }
    pub fn flag(&self, k: &str) -> bool {
        self.get(k) == "yes"
    }
    pub fn set(&mut self, k: &str, v: &str) {
        self.values.insert(k.into(), v.into());
        let body: String = self.values.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        if let Some(dir) = std::path::Path::new(&self.path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.path, body);
        let public: String = self.values.iter().filter(|(k, _)| !SECRET_KEYS.contains(&k.as_str()))
            .map(|(k, v)| format!("{k}={v}\n")).collect();
        let _ = std::fs::write(&self.backup, format!("# Kindle Linux settings (backup copy; safe to delete)\n{public}"));
    }
}

// --- Wi-Fi ------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Network {
    pub ssid: String,
    pub signal: i32,
    pub secure: bool,
}

#[derive(Clone)]
pub struct Wifi {
    sim: bool,
    conf: String,
    dhcp: Arc<Mutex<Option<Child>>>,
}

impl Wifi {
    pub fn new(sim: bool) -> Wifi {
        let conf = format!("{STATE}/wpa_supplicant.conf");
        if !sim && !std::path::Path::new(&conf).exists() {
            let _ = std::fs::create_dir_all(STATE);
            let _ = std::fs::write(&conf, format!("ctrl_interface={WPA_CTRL}\nupdate_config=1\ncountry=US\n"));
        }
        Wifi { sim, conf, dhcp: Arc::new(Mutex::new(None)) }
    }

    fn cli(&self, args: &[&str]) -> String {
        let mut a = vec!["wpa_cli", "-p", WPA_CTRL, "-i", IFACE];
        a.extend_from_slice(args);
        run(&a, false).trim().to_string()
    }

    /// Our wpa_supplicant is alive. (Its control socket can outlive it, so check the process too.)
    pub fn running(&self) -> bool {
        if self.sim {
            return true;
        }
        if !std::path::Path::new(&format!("{WPA_CTRL}/{IFACE}")).exists() {
            return false;
        }
        match std::fs::read_to_string("/run/wpa_supplicant.pid").ok().and_then(|p| p.trim().parse::<u32>().ok()) {
            Some(pid) => std::path::Path::new(&format!("/proc/{pid}")).exists(),
            None => true,
        }
    }

    /// Nudge a running supplicant to find the network again and renew the address.
    pub fn reconnect(&self) {
        if self.sim {
            return;
        }
        log("wifi: reconnecting");
        self.cli(&["reassociate"]);
        self.start_dhcp();
    }

    fn iface_present() -> bool {
        std::path::Path::new(&format!("/sys/class/net/{IFACE}")).exists()
    }

    /// Bring up wlan0 and our wpa_supplicant. Every step is logged to kos/kos.log.
    pub fn start(&self) -> Result<(), String> {
        if self.sim || self.running() {
            return Ok(());
        }
        log("wifi: starting");
        if !Self::iface_present() {
            // The driver/chip may have been powered down with Amazon's wifi services: try to bring it back.
            log("wifi: wlan0 missing, starting Amazon's wifim job and loading ath6kl_sdio");
            log(&format!("wifi: start wifim -> {}", run(&["start", "wifim"], true).trim()));
            log(&format!("wifi: modprobe -> {}", run(&["modprobe", "ath6kl_sdio"], true).trim()));
            for _ in 0..40 {
                if Self::iface_present() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            if !Self::iface_present() {
                let ifs = run(&["ls", "/sys/class/net"], false);
                log(&format!("wifi: still no wlan0; interfaces: {}", ifs.trim()));
                return Err(format!("No Wi-Fi interface (wlan0). Found: {}", ifs.split_whitespace().collect::<Vec<_>>().join(", ")));
            }
        }
        // rfkill soft-block, if the kernel exposes one
        if let Ok(rd) = std::fs::read_dir("/sys/class/rfkill") {
            for e in rd.flatten() {
                let _ = std::fs::write(e.path().join("soft"), "0");
            }
        }
        let up = run(&["ip", "link", "set", IFACE, "up"], false);
        log(&format!("wifi: ip link up -> {}", if up.trim().is_empty() { "ok" } else { up.trim() }));
        let _ = std::fs::create_dir_all(WPA_CTRL);
        let out = run(&["wpa_supplicant", "-B", "-D", "nl80211", "-i", IFACE, "-c", &self.conf,
                        "-P", "/run/wpa_supplicant.pid"], false);
        log(&format!("wifi: wpa_supplicant -> {}", out.trim()));
        for _ in 0..20 {
            if self.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if !self.running() {
            let first = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string();
            return Err(format!("wpa_supplicant didn't start: {first}"));
        }
        log("wifi: wpa_supplicant running");
        self.start_dhcp();
        Ok(())
    }

    pub fn start_dhcp(&self) {
        if self.sim {
            return;
        }
        let mut d = self.dhcp.lock().unwrap();
        if let Some(c) = d.as_mut() {
            if c.try_wait().ok().flatten().is_none() {
                return;
            }
        }
        *d = Command::new("udhcpc")
            .args(["-i", IFACE, "-f", "-S", "-R", "-t", "8", "-T", "3", "-s", "/opt/kos/udhcpc.script"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok();
    }

    pub fn stop(&self) {
        if self.sim {
            return;
        }
        self.cli(&["terminate"]);
        if let Some(mut c) = self.dhcp.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        // wait for wpa_supplicant to exit, so a later start() really starts it again
        for _ in 0..20 {
            if !self.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Some(pid) = std::fs::read_to_string("/run/wpa_supplicant.pid").ok().and_then(|p| p.trim().parse::<i32>().ok()) {
            if std::path::Path::new(&format!("/proc/{pid}")).exists() {
                unsafe { libc::kill(pid, libc::SIGTERM) };
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        let _ = std::fs::remove_file(format!("{WPA_CTRL}/{IFACE}"));
        let _ = std::fs::remove_file("/run/wpa_supplicant.pid");
        run(&["ip", "addr", "flush", "dev", IFACE], false);
        run(&["ip", "link", "set", IFACE, "down"], false);
    }

    pub fn status(&self) -> BTreeMap<String, String> {
        let mut st = BTreeMap::new();
        if self.sim {
            st.insert("wpa_state".into(), "COMPLETED".into());
            st.insert("ssid".into(), "HomeNetwork".into());
            st.insert("ip_address".into(), "192.168.1.50".into());
            return st;
        }
        for line in self.cli(&["status"]).lines() {
            if let Some((k, v)) = line.split_once('=') {
                st.insert(k.into(), v.into());
            }
        }
        if let Some(ip) = ip_address(false) {
            st.insert("ip_address".into(), ip);
        }
        st
    }

    pub fn scan(&self) -> Vec<Network> {
        if self.sim {
            return vec![
                Network { ssid: "HomeNetwork".into(), signal: -45, secure: true },
                Network { ssid: "Cafe Guest".into(), signal: -67, secure: false },
                Network { ssid: "Neighbour 5G".into(), signal: -80, secure: true },
            ];
        }
        self.cli(&["scan"]);
        std::thread::sleep(Duration::from_secs(3));
        let mut best: BTreeMap<String, Network> = BTreeMap::new();
        for line in self.cli(&["scan_results"]).lines().skip(1) {
            let p: Vec<&str> = line.split('\t').collect();
            if p.len() < 5 || p[4].is_empty() {
                continue;
            }
            let n = Network { ssid: p[4].into(), signal: p[2].parse().unwrap_or(-100),
                              secure: p[3].contains("WPA") || p[3].contains("WEP") };
            match best.get(&n.ssid) {
                Some(o) if o.signal >= n.signal => {}
                _ => {
                    best.insert(n.ssid.clone(), n);
                }
            }
        }
        let mut v: Vec<Network> = best.into_values().collect();
        v.sort_by_key(|n| -n.signal);
        v
    }

    fn saved(&self) -> Vec<(String, String)> {
        self.cli(&["list_networks"]).lines().skip(1)
            .filter_map(|l| {
                let p: Vec<&str> = l.split('\t').collect();
                (p.len() >= 2).then(|| (p[0].to_string(), p[1].to_string()))
            }).collect()
    }

    pub fn connect(&self, ssid: &str, password: Option<&str>) {
        if self.sim {
            return;
        }
        for (id, s) in self.saved() {
            if s == ssid {
                self.cli(&["remove_network", &id]);
            }
        }
        let id = self.cli(&["add_network"]).lines().last().unwrap_or("0").trim().to_string();
        let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        self.cli(&["set_network", &id, "ssid", &q(ssid)]);
        match password {
            Some(pw) if !pw.is_empty() => {
                self.cli(&["set_network", &id, "psk", &q(pw)]);
            }
            _ => {
                self.cli(&["set_network", &id, "key_mgmt", "NONE"]);
            }
        }
        self.cli(&["enable_network", &id]);
        self.cli(&["select_network", &id]);
        self.cli(&["save_config"]);
        self.start_dhcp();
    }

    pub fn forget(&self, ssid: &str) {
        for (id, s) in self.saved() {
            if s == ssid {
                self.cli(&["remove_network", &id]);
            }
        }
        self.cli(&["save_config"]);
    }
}

// --- SSH --------------------------------------------------------------------------
pub struct Ssh {
    sim: bool,
}

const DROPBEAR_PID: &str = "/run/dropbear.pid";

impl Ssh {
    pub fn running(&self) -> bool {
        !self.sim && read(DROPBEAR_PID).map_or(false, |p| std::path::Path::new(&format!("/proc/{p}")).exists())
    }

    pub fn has_password(&self) -> bool {
        read("/etc/shadow").unwrap_or_default().lines().find(|l| l.starts_with("root:"))
            .map_or(false, |l| !matches!(l.split(':').nth(1), Some("") | Some("*") | Some("!") | Some("!!") | None))
    }

    pub fn set_password(&self, pw: &str) -> Result<(), String> {
        use std::io::Write;
        if self.sim {
            return Ok(());
        }
        let mut child = Command::new("chpasswd").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().map_err(|e| e.to_string())?;
        child.stdin.take().unwrap().write_all(format!("root:{pw}\n").as_bytes()).map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        // keep the password across image updates (the launcher puts it back into /etc/shadow)
        if let Some(line) = std::fs::read_to_string("/etc/shadow").ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("root:")).map(String::from)) {
            let p = format!("{STATE}/root-shadow");
            let _ = std::fs::write(&p, format!("{line}\n"));
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn start(&self) {
        if self.sim || self.running() {
            return;
        }
        let _ = std::fs::create_dir_all("/etc/dropbear");
        run(&["dropbear", "-R", "-p", "22", "-P", DROPBEAR_PID], false);
        // Kindle's firewall drops inbound connections; open port 22 while SSH runs.
        if !ok(&["iptables", "-C", "INPUT", "-p", "tcp", "--dport", "22", "-j", "ACCEPT"], true) {
            run(&["iptables", "-I", "INPUT", "-p", "tcp", "--dport", "22", "-j", "ACCEPT"], true);
        }
    }

    pub fn stop(&self) {
        if self.sim {
            return;
        }
        if let Some(pid) = read(DROPBEAR_PID) {
            run(&["kill", &pid], false);
        }
        run(&["iptables", "-D", "INPUT", "-p", "tcp", "--dport", "22", "-j", "ACCEPT"], true);
    }
}

// --- Power: kernel suspend + powerd/HAL event watcher ---------------------------------
pub struct Power {
    sim: bool,
}

impl Power {
    /// Suspend straight through the kernel (blocks until the power button wakes the device).
    pub fn suspend_now(&self) -> Result<(), String> {
        if self.sim {
            return Ok(());
        }
        std::fs::write("/sys/power/state", "mem").map_err(|e| e.to_string())
    }

    /// Long-running `lipc-wait-event`s printing every powerd and HAL event, one per line.
    /// (Used to catch power-button presses when the input device is grabbed by powerd.)
    pub fn watcher(&self) -> Option<Child> {
        if self.sim {
            return None;
        }
        Command::new("chroot")
            .args([HOST, "sh", "-c",
                   "lipc-wait-event -m com.lab126.powerd '*' & lipc-wait-event -m com.lab126.hal '*' & wait"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()
    }
}

// --- Sys: shared state handed to apps ------------------------------------------------
#[derive(Default, Clone)]
pub struct Status {
    pub battery: Option<u32>,
    pub charging: bool,
    pub wifi: bool,    // connected (has an address)
    pub wifi_on: bool, // switched on (maybe still connecting)
    pub ssh: bool,
}

pub struct Sys {
    pub sim: bool,
    pub us: String,
    pub config: Config,
    pub wifi: Wifi,
    pub ssh: Ssh,
    pub power: Power,
    pub keyboard_visible: bool,
    pub full_every_changed: Option<u32>,
    pub sleep_requested: bool,
    /// Apps that must not be interrupted by auto-sleep (e.g. the upload page) set this.
    pub keep_awake: bool,
    /// Screen position of the gesture currently being dispatched.
    pub last_touch: (i32, i32),
    /// Latest weather data, refreshed in the background.
    pub dash: Arc<Mutex<crate::dashboard::Dash>>,
    pub dash_refresh_now: bool,
    status: (Option<Instant>, Status),
}

impl Sys {
    pub fn new(sim: bool) -> Sys {
        Sys {
            sim,
            us: userstore(sim),
            config: Config::load(sim),
            wifi: Wifi::new(sim),
            ssh: Ssh { sim },
            power: Power { sim },
            keyboard_visible: true,
            full_every_changed: None,
            sleep_requested: false,
            keep_awake: false,
            last_touch: (0, 0),
            dash: Arc::new(Mutex::new(Default::default())),
            dash_refresh_now: false,
            status: (None, Status::default()),
        }
    }

    pub fn status(&mut self) -> Status {
        if self.status.0.map_or(true, |t| t.elapsed() > Duration::from_secs(8)) {
            let b = battery(self.sim);
            let wifi_on = self.wifi.running();
            let wifi = wifi_on && ip_address(self.sim).is_some();
            self.status = (Some(Instant::now()), Status { battery: b.capacity, charging: b.charging(), wifi, wifi_on,
                                                           ssh: self.ssh.running() });
        }
        self.status.1.clone()
    }

    pub fn invalidate_status(&mut self) {
        self.status.0 = None;
    }

    /// Ask the host launcher to do something after we exit (e.g. open KOReader).
    pub fn request(&self, action: &str, arg: &str) {
        let path = format!("{}/kos/request", self.us);
        let _ = std::fs::create_dir_all(format!("{}/kos", self.us));
        let _ = std::fs::write(path, format!("{action}\n{arg}\n"));
    }
}
