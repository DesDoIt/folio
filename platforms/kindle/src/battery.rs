//! Battery: CPU speed management, a battery log (samples over time, awake vs asleep) and a
//! Battery screen with history and estimates.

use crate::canvas::*;
use crate::sys::{self, Sys};
use crate::ui::*;

const GOVERNOR: &str = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";

/// Switch the CPU frequency governor ("performance" while interacting, "ondemand" when idle).
pub fn set_governor(sim: bool, g: &str) {
    if !sim {
        let _ = std::fs::write(GOVERNOR, g);
    }
}

#[derive(Clone, Copy)]
pub struct Sample {
    pub t: i64,
    pub pct: u32,
    pub charging: bool,
    pub awake: bool,
}

fn log_path(sys: &Sys) -> String {
    format!("{}/kos/battery.log", sys.us)
}

fn now() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// Append a sample ("awake" or "asleep" describes the state that starts now).
pub fn record(sys: &Sys, awake: bool) {
    let b = sys::battery(sys.sim);
    let Some(pct) = b.capacity else { return };
    let path = log_path(sys);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    text.push_str(&format!("{}\t{pct}\t{}\t{}\n", now(), if b.charging() { "c" } else { "d" }, if awake { "awake" } else { "asleep" }));
    // keep roughly the last few weeks
    let lines: Vec<&str> = text.lines().collect();
    let keep = if lines.len() > 3000 { &lines[lines.len() - 2500..] } else { &lines[..] };
    let _ = std::fs::create_dir_all(format!("{}/kos", sys.us));
    let _ = std::fs::write(&path, keep.join("\n") + "\n");
}

pub fn load(sys: &Sys) -> Vec<Sample> {
    std::fs::read_to_string(log_path(sys)).unwrap_or_default().lines().filter_map(|l| {
        let p: Vec<&str> = l.split('\t').collect();
        if p.len() < 4 {
            return None;
        }
        Some(Sample { t: p[0].parse().ok()?, pct: p[1].parse().ok()?, charging: p[2] == "c", awake: p[3] == "awake" })
    }).collect()
}

/// Average drain in %/hour while awake and while asleep (discharging periods only).
pub fn drain_rates(s: &[Sample]) -> (Option<f64>, Option<f64>) {
    let (mut aw, mut awh, mut sl, mut slh) = (0.0, 0.0, 0.0, 0.0);
    for w in s.windows(2) {
        let (a, b) = (w[0], w[1]);
        let hours = (b.t - a.t) as f64 / 3600.0;
        if a.charging || b.charging || hours <= 0.0 || hours > 48.0 || b.pct > a.pct {
            continue;
        }
        let used = (a.pct - b.pct) as f64;
        if a.awake {
            aw += used;
            awh += hours;
        } else {
            sl += used;
            slh += hours;
        }
    }
    let rate = |u: f64, h: f64, min_h: f64| if h >= min_h && u > 0.0 { Some(u / h) } else { None };
    (rate(aw, awh, 0.5), rate(sl, slh, 2.0))
}

pub struct BatteryApp;

impl App for BatteryApp {
    fn title(&self) -> String {
        "Battery".into()
    }

    fn render(&mut self, cx: &mut Ctx, a: Rect, sys: &mut Sys) {
        let b = sys::battery(sys.sim);
        let pct = b.capacity.unwrap_or(0);
        // big gauge
        let g = Rect::new(a.x0 + 40, a.y0 + 24, a.x0 + 260, a.y0 + 104);
        cx.c.round_rect(g, 10, WHITE, 3, BLACK);
        cx.c.fill(Rect::new(g.x1, g.cy() - 14, g.x1 + 10, g.cy() + 14), BLACK);
        let fill_w = (g.w() - 12) * pct.min(100) as i32 / 100;
        cx.c.fill(Rect::new(g.x0 + 6, g.y0 + 6, g.x0 + 6 + fill_w, g.y1 - 6), DARK);
        cx.bold(a.x0 + 300, a.y0 + 34, &format!("{pct}%"), 44, BLACK, Anchor::LT);
        cx.text(a.x0 + 300, a.y0 + 88, &b.status, 18, DARK, Anchor::LT);
        let samples = load(sys);
        let (awake, asleep) = drain_rates(&samples);
        let mut y = a.y0 + 140;
        let est = |rate: Option<f64>, unit_h: f64, label: &str| match rate {
            Some(r) if r > 0.0 => {
                let v = pct as f64 / r / unit_h;
                format!("{label}: about {v:.0} {}  ({r:.1}%/h)", if unit_h >= 24.0 { "days" } else { "hours" })
            }
            _ => format!("{label}: not enough data yet"),
        };
        cx.text(a.x0 + 24, y, &est(awake, 1.0, "Reading"), 19, BLACK, Anchor::LT);
        y += 34;
        cx.text(a.x0 + 24, y, &est(asleep, 24.0, "Standby"), 19, BLACK, Anchor::LT);
        y += 50;
        // 3-day graph
        cx.bold(a.x0 + 24, y, "Last 3 days", 18, BLACK, Anchor::LT);
        y += 34;
        let chart = Rect::new(a.x0 + 50, y, a.x1 - 24, y + 300);
        cx.c.outline(chart, 1, DARK);
        for p in [25, 50, 75] {
            let gy = chart.y1 - chart.h() * p / 100;
            cx.c.hline(chart.x0, chart.x1, gy, 1, LIGHT);
            cx.text(chart.x0 - 6, gy, &format!("{p}"), 12, DARK, Anchor::RM);
        }
        let t1 = now();
        let t0 = t1 - 3 * 86400;
        let xs = |t: i64| chart.x0 + ((t - t0) as f64 / (t1 - t0) as f64 * chart.w() as f64) as i32;
        let ys = |p: u32| chart.y1 - (chart.h() as f64 * p.min(100) as f64 / 100.0) as i32;
        let recent: Vec<&Sample> = samples.iter().filter(|s| s.t >= t0).collect();
        for w in recent.windows(2) {
            let (s0, s1) = (w[0], w[1]);
            let (x0, x1) = (xs(s0.t), xs(s1.t));
            if !s0.awake {
                cx.c.fill(Rect::new(x0, chart.y0 + 1, x1.max(x0 + 1), chart.y1), 235); // asleep: light band
            }
            // line segment
            let (y0, y1) = (ys(s0.pct), ys(s1.pct));
            let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
            for k in 0..=steps {
                let x = x0 + (x1 - x0) * k / steps;
                let yy = y0 + (y1 - y0) * k / steps;
                cx.c.fill(Rect::new(x - 1, yy - 1, x + 2, yy + 2), BLACK);
            }
        }
        for d in 0..=3 {
            let x = xs(t0 + d * 86400);
            cx.text(x, chart.y1 + 16, &sys::format_time(t0 + d * 86400, "%a"), 12, DARK, Anchor::MM);
        }
        if recent.len() < 2 {
            cx.text(chart.cx(), chart.cy(), "Collecting data…", 18, DARK, Anchor::MM);
        }
        y = chart.y1 + 40;
        cx.text(a.x0 + 24, y, "Shaded = asleep. Samples every 10 min and at sleep/wake.", 14, DARK, Anchor::LT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain() {
        let h = 3600;
        let s = vec![
            Sample { t: 0, pct: 100, charging: false, awake: true },
            Sample { t: h, pct: 95, charging: false, awake: false },   // 5%/h awake
            Sample { t: 11 * h, pct: 93, charging: false, awake: true }, // 0.2%/h asleep
        ];
        let (a, sl) = drain_rates(&s);
        assert!((a.unwrap() - 5.0).abs() < 0.01);
        assert!((sl.unwrap() - 0.2).abs() < 0.01);
    }
}
