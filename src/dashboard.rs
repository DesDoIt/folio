//! Weather dashboard (Open-Meteo, no account needed), shown on the sleep screen and the home screen.
//! Data is refreshed in the background while awake, because the device suspends shortly after the
//! sleep screen is drawn.

use crate::canvas::*;
use crate::fb::{Display, Update};
use crate::getbooks::http_get;
use crate::json;
use crate::sys::{self, Sys};

#[derive(Clone, Default)]
pub struct Day {
    pub name: String,
    pub code: i32,
    pub tmax: f64,
    pub tmin: f64,
    pub rain: i32,
}

#[derive(Clone, Default)]
pub struct Weather {
    pub place: String,
    pub temp: f64,
    pub feels: f64,
    pub humidity: i32,
    pub code: i32,
    pub wind: f64,
    pub sunrise: String,
    pub sunset: String,
    pub days: Vec<Day>,
}

#[derive(Clone, Default)]
pub struct Dash {
    pub weather: Option<Weather>,
    pub updated: Option<i64>,
    pub error: Option<String>,
}

/// WMO weather code -> (short description, glyph available in DejaVu Sans)
pub fn describe(code: i32) -> (&'static str, &'static str) {
    match code {
        0 => ("Clear", "☀"),
        1 | 2 => ("Partly cloudy", "☀"),
        3 => ("Overcast", "☁"),
        45 | 48 => ("Fog", "☁"),
        51..=57 => ("Drizzle", "☂"),
        61..=67 | 80..=82 => ("Rain", "☂"),
        71..=77 | 85 | 86 => ("Snow", "❄"),
        95..=99 => ("Thunderstorm", "⚡"),
        _ => ("—", "☁"),
    }
}

fn now() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// Look up a place name: returns (lat, lon, "City, Country").
pub fn geocode(name: &str) -> Result<(f64, f64, String), String> {
    let q: String = name.trim().replace(' ', "+");
    let body = http_get(&format!("https://geocoding-api.open-meteo.com/v1/search?name={q}&count=1&format=json"))?;
    let j = json::parse(&body).map_err(|e| e.to_string())?;
    let r = j.get("results").arr().first().ok_or_else(|| format!("No place called “{}” found", name.trim()))?;
    let lat = r.get("latitude").num().ok_or("bad geocoding answer")?;
    let lon = r.get("longitude").num().ok_or("bad geocoding answer")?;
    let label = match (r.get("name").str(), r.get("country").str()) {
        (Some(n), Some(c)) => format!("{n}, {c}"),
        (Some(n), None) => n.to_string(),
        _ => name.trim().to_string(),
    };
    Ok((lat, lon, label))
}

pub fn fetch_weather(lat: f64, lon: f64, place: &str) -> Result<Weather, String> {
    let url = format!("https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
        &current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m\
        &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset\
        &timezone=auto&forecast_days=5");
    let j = json::parse(&http_get(&url)?).map_err(|e| e.to_string())?;
    let cur = j.get("current");
    let d = j.get("daily");
    let num = |k: &str, i: usize| d.get(k).arr().get(i).and_then(|v| v.num()).unwrap_or(0.0);
    let mut days = vec![];
    for i in 0..d.get("time").arr().len() {
        let date = d.get("time").arr()[i].str().unwrap_or("");
        days.push(Day {
            name: weekday_of(date),
            code: num("weather_code", i) as i32,
            tmax: num("temperature_2m_max", i),
            tmin: num("temperature_2m_min", i),
            rain: num("precipitation_probability_max", i) as i32,
        });
    }
    // sunrise/sunset come as local "2026-11-22T07:31"
    let hm = |k: &str| d.get(k).arr().first().and_then(|v| v.str()).and_then(|s| s.split('T').nth(1)).unwrap_or("").to_string();
    Ok(Weather {
        place: place.to_string(),
        temp: cur.get("temperature_2m").num().unwrap_or(0.0),
        feels: cur.get("apparent_temperature").num().unwrap_or(0.0),
        humidity: cur.get("relative_humidity_2m").num().unwrap_or(0.0) as i32,
        code: cur.get("weather_code").num().unwrap_or(0.0) as i32,
        wind: cur.get("wind_speed_10m").num().unwrap_or(0.0),
        sunrise: hm("sunrise"),
        sunset: hm("sunset"),
        days,
    })
}

/// "2026-11-22" -> "Sun"
fn weekday_of(date: &str) -> String {
    let p: Vec<i32> = date.split('-').filter_map(|x| x.parse().ok()).collect();
    if p.len() != 3 {
        return String::new();
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = p[0] - 1900;
    tm.tm_mon = p[1] - 1;
    tm.tm_mday = p[2];
    tm.tm_hour = 12;
    let t = unsafe { libc::mktime(&mut tm) } as i64;
    sys::format_time(t, "%a")
}

/// Refresh the weather (blocking; run on a thread).
pub fn refresh(loc: (f64, f64, String)) -> Dash {
    let mut d = Dash { updated: Some(now()), ..Default::default() };
    match fetch_weather(loc.0, loc.1, &loc.2) {
        Ok(w) => d.weather = Some(w),
        Err(e) => d.error = Some(e),
    }
    d
}

pub fn configured_location(sys: &Sys) -> Option<(f64, f64, String)> {
    let lat = sys.config.get("weather_lat").parse().ok()?;
    let lon = sys.config.get("weather_lon").parse().ok()?;
    Some((lat, lon, sys.config.get("weather_place").to_string()))
}

/// Draw the weather sleep screen.
pub fn draw_sleep(disp: &mut dyn Display, fonts: &mut Fonts, d: &Dash, battery: Option<u32>) {
    let (w, h) = disp.size();
    let mut c = Canvas::new(w, h);
    let t = now();
    c.text(fonts, 36, 40, &sys::format_time(t, "%A"), Face::Bold, 46, BLACK, Anchor::LT);
    c.text(fonts, 36, 100, &sys::format_time(t, "%d %B %Y"), Face::Sans, 26, DARK, Anchor::LT);
    let mut y = 160;
    c.hline(36, w - 36, y, 2, BLACK);
    y += 30;
    match &d.weather {
        Some(wx) => {
            let (desc, glyph) = describe(wx.code);
            c.text(fonts, 36, y, glyph, Face::Sans, 120, BLACK, Anchor::LT);
            c.text(fonts, 210, y, &format!("{:.0}°", wx.temp), Face::Bold, 96, BLACK, Anchor::LT);
            y += 140;
            c.text(fonts, 36, y, desc, Face::Bold, 32, BLACK, Anchor::LT);
            let place = fonts.ellipsize(&wx.place, Face::Sans, 20, w - 72);
            c.text(fonts, 36, y + 46, &place, Face::Sans, 20, DARK, Anchor::LT);
            y += 96;
            let details = [
                ("Feels like", format!("{:.0}°", wx.feels)),
                ("Humidity", format!("{}%", wx.humidity)),
                ("Wind", format!("{:.0} km/h", wx.wind)),
            ];
            let cw = (w - 72) / 3;
            for (i, (k, v)) in details.iter().enumerate() {
                let x = 36 + i as i32 * cw;
                c.text(fonts, x, y, k, Face::Sans, 16, DARK, Anchor::LT);
                c.text(fonts, x, y + 24, v, Face::Bold, 26, BLACK, Anchor::LT);
            }
            y += 70;
            if !wx.sunrise.is_empty() {
                c.text(fonts, 36, y, &format!("☀ Sunrise {}    ☾ Sunset {}", wx.sunrise, wx.sunset), Face::Sans, 18, DARK, Anchor::LT);
                y += 40;
            }
            c.hline(36, w - 36, y, 2, BLACK);
            y += 24;
            let n = wx.days.len().min(5).max(1) as i32;
            let cw = (w - 72) / n;
            for (i, day) in wx.days.iter().take(5).enumerate() {
                let x = 36 + i as i32 * cw + cw / 2;
                let name = if i == 0 { "Today".to_string() } else { day.name.clone() };
                c.text(fonts, x, y, &name, Face::Bold, 18, BLACK, Anchor::MT);
                c.text(fonts, x, y + 30, describe(day.code).1, Face::Sans, 40, BLACK, Anchor::MT);
                c.text(fonts, x, y + 84, &format!("{:.0}°", day.tmax), Face::Bold, 22, BLACK, Anchor::MT);
                c.text(fonts, x, y + 112, &format!("{:.0}°", day.tmin), Face::Sans, 18, DARK, Anchor::MT);
                c.text(fonts, x, y + 140, &format!("☂ {}%", day.rain), Face::Sans, 15, DARK, Anchor::MT);
            }
        }
        None => {
            c.text(fonts, w / 2, y + 120, "Sleeping", Face::Bold, 54, BLACK, Anchor::MM);
            c.text(fonts, w / 2, y + 190, "Set a weather location in Settings", Face::Sans, 20, DARK, Anchor::MM);
        }
    }
    let updated = d.updated.map_or("not updated yet".into(), |u| format!("updated {}", sys::format_time(u, "%H:%M")));
    let foot = format!("Battery {}%  ·  {}  ·  Press power to wake", battery.map_or("?".into(), |b| b.to_string()), updated);
    if d.weather.is_some() {
        c.text(fonts, w / 2, h - 66, "Weather data: Open-Meteo.com (CC BY 4.0)", Face::Sans, 13, DARK, Anchor::MM);
    }
    c.hline(36, w - 36, h - 48, 1, LIGHT);
    c.text(fonts, w / 2, h - 28, &foot, Face::Sans, 15, DARK, Anchor::MM);
    disp.present(&c, Rect::new(0, 0, w, h), Update::Full);
}
