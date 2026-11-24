//! What NIRUTODAY shows and where it comes from. Each source has a cache
//! key, a request, a refresh interval and a parser into plain data; the
//! parsers are pure, so the tests feed them saved answers.
use crate::config::Config;
use chrono::{DateTime, Local, NaiveDate};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Weather,
    Warnings,
    Vma,
    Kris,
    News(usize),
    /// 0 today, 1 tomorrow.
    Prices(u8),
    Day,
    Traffic,
}

impl Kind {
    pub fn all(cfg: &Config) -> Vec<Kind> {
        let mut v = vec![
            Kind::Vma,
            Kind::Weather,
            Kind::Warnings,
            Kind::Kris,
            Kind::Day,
            Kind::Prices(0),
            Kind::Prices(1),
        ];
        v.extend((0..cfg.news.len()).map(Kind::News));
        if cfg.traffic && !cfg.traffic_key.trim().is_empty() {
            v.push(Kind::Traffic);
        }
        v
    }

    /// Seconds between refreshes.
    pub fn interval(&self) -> i64 {
        match self {
            Kind::Vma => 120,
            Kind::News(_) | Kind::Traffic => 300,
            Kind::Warnings | Kind::Kris => 600,
            Kind::Weather => 1800,
            Kind::Prices(0) => 3 * 3600,
            Kind::Prices(_) => 1800,
            Kind::Day => 6 * 3600,
        }
    }

    /// Does this source keep running while the window is hidden? Only
    /// what can raise a notice.
    pub fn while_hidden(&self) -> bool {
        matches!(self, Kind::Vma | Kind::Warnings)
    }

    pub fn key(&self, cfg: &Config, today: NaiveDate) -> String {
        match self {
            Kind::Weather => "smhi-forecast.json".into(),
            Kind::Warnings => "smhi-warnings.json".into(),
            Kind::Vma => format!("vma-{}.json", cfg.county),
            Kind::Kris => "krisinformation.json".into(),
            Kind::News(i) => format!(
                "sr-{}.xml",
                cfg.news.get(*i).map(|n| n.program).unwrap_or(0)
            ),
            Kind::Prices(d) => {
                let day = today + chrono::Duration::days(i64::from(*d));
                format!("prices-{}-{}.json", day.format("%Y-%m-%d"), cfg.price_area)
            }
            Kind::Day => format!("day-{}.json", today.format("%Y-%m-%d")),
            Kind::Traffic => "traffic.json".into(),
        }
    }

    pub fn url(&self, cfg: &Config, today: NaiveDate) -> String {
        match self {
            Kind::Weather => format!(
                "https://opendata-download-metfcst.smhi.se/api/category/snow1g/version/1/geotype/point/lon/{:.4}/lat/{:.4}/data.json",
                cfg.longitude, cfg.latitude
            ),
            Kind::Warnings => "https://opendata-download-warnings.smhi.se/ibww/api/version/1/warning.json".into(),
            Kind::Vma => format!("https://vmaapi.sr.se/api/v2/alerts/{:02}", cfg.county),
            Kind::Kris => "https://api.krisinformation.se/v3/news?days=3".into(),
            Kind::News(i) => format!("https://api.sr.se/api/rss/program/{}", cfg.news.get(*i).map(|n| n.program).unwrap_or(0)),
            Kind::Prices(d) => {
                let day = today + chrono::Duration::days(i64::from(*d));
                format!(
                    "https://www.elprisetjustnu.se/api/v1/prices/{}/{}_{}.json",
                    day.format("%Y"),
                    day.format("%m-%d"),
                    cfg.price_area
                )
            }
            Kind::Day => format!("https://sholiday.faboul.se/dagar/v2.1/{}", today.format("%Y/%m/%d")),
            Kind::Traffic => "https://api.trafikinfo.trafikverket.se/v2/data.json".into(),
        }
    }

    pub fn body(&self, cfg: &Config) -> Option<String> {
        (*self == Kind::Traffic).then(|| traffic_request(cfg))
    }
}

// ---------------------------------------------------------------- weather

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    /// UTC seconds at the end of the interval the values describe.
    pub at: i64,
    /// Length of that interval in hours (1, later 3, 6, 12).
    pub span: f64,
    pub temp: f64,
    pub symbol: u8,
    /// Mean precipitation, mm per hour.
    pub rain: f64,
    pub wind: f64,
    pub gust: f64,
    pub wind_dir: f64,
    pub humidity: f64,
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(f64::NAN)
}

fn ts(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp())
}

pub fn parse_weather(text: &str) -> Result<Vec<Hour>, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "SMHI: not JSON")?;
    let series = v
        .get("timeSeries")
        .and_then(Value::as_array)
        .ok_or("SMHI: no forecast")?;
    let mut out = Vec::new();
    for e in series {
        let Some(at) = e.get("time").and_then(Value::as_str).and_then(ts) else {
            continue;
        };
        let from = e
            .get("intervalParametersStartTime")
            .and_then(Value::as_str)
            .and_then(ts)
            .unwrap_or(at - 3600);
        let d = e.get("data").unwrap_or(&Value::Null);
        let temp = num(d, "air_temperature");
        if temp.is_nan() {
            continue;
        }
        out.push(Hour {
            at,
            span: ((at - from) as f64 / 3600.0).max(1.0),
            temp,
            symbol: num(d, "symbol_code").clamp(0.0, 27.0) as u8,
            rain: num(d, "precipitation_amount_mean").max(0.0),
            wind: num(d, "wind_speed"),
            gust: num(d, "wind_speed_of_gust"),
            wind_dir: num(d, "wind_from_direction"),
            humidity: num(d, "relative_humidity"),
        });
    }
    if out.is_empty() {
        return Err("SMHI: empty forecast".into());
    }
    Ok(out)
}

/// SMHI's "känns som" (wind chill) for cold, windy weather; otherwise the
/// temperature.
pub fn feels_like(temp: f64, wind: f64) -> f64 {
    if temp > 10.0 || wind.is_nan() || wind <= 1.3 {
        return temp;
    }
    let v = (wind * 3.6).powf(0.16);
    13.12 + 0.6215 * temp - 11.37 * v + 0.3965 * temp * v
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub low: f64,
    pub high: f64,
    pub symbol: u8,
    pub rain: f64,
    pub wind: f64,
}

/// Days from the forecast: low, high, total precipitation, strongest mean
/// wind, and the symbol nearest midday (the worst one when it rains).
pub fn days(hours: &[Hour], to_local: impl Fn(i64) -> chrono::NaiveDateTime) -> Vec<Day> {
    use chrono::Timelike;
    let mut out: Vec<Day> = Vec::new();
    let mut best_noon: Vec<i64> = Vec::new();
    for h in hours {
        let local = to_local(h.at - 1);
        let date = local.date();
        if out.last().map(|d| d.date) != Some(date) {
            out.push(Day {
                date,
                low: h.temp,
                high: h.temp,
                symbol: h.symbol,
                rain: 0.0,
                wind: 0.0,
            });
            best_noon.push(i64::MAX);
        }
        let d = out.last_mut().unwrap();
        d.low = d.low.min(h.temp);
        d.high = d.high.max(h.temp);
        d.rain += h.rain * h.span;
        if h.wind.is_finite() {
            d.wind = d.wind.max(h.wind);
        }
        let dist = (i64::from(local.hour()) - 12).abs();
        let n = best_noon.last_mut().unwrap();
        if dist < *n {
            *n = dist;
            d.symbol = h.symbol;
        }
    }
    out
}

// ---------------------------------------------------------------- warnings

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Message,
    Yellow,
    Orange,
    Red,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub id: String,
    pub level: Level,
    pub event: String,
    pub area: String,
    pub text: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
}

fn sv(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.get("sv").or(Some(x)))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub fn parse_warnings(text: &str, county: u32) -> Result<Vec<Warning>, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "SMHI warnings: not JSON")?;
    let list = v.as_array().ok_or("SMHI warnings: unexpected answer")?;
    let mut out = Vec::new();
    for w in list {
        let event = sv(w, "event");
        for a in w
            .get("warningAreas")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let ours = a
                .get("affectedAreas")
                .and_then(Value::as_array)
                .is_some_and(|l| {
                    l.iter()
                        .any(|x| x.get("id").and_then(Value::as_u64) == Some(u64::from(county)))
                });
            if !ours {
                continue;
            }
            let level = match a
                .get("warningLevel")
                .and_then(|l| l.get("code"))
                .and_then(Value::as_str)
            {
                Some("RED") => Level::Red,
                Some("ORANGE") => Level::Orange,
                Some("YELLOW") => Level::Yellow,
                _ => Level::Message,
            };
            let text = a
                .get("descriptions")
                .and_then(Value::as_array)
                .and_then(|d| {
                    d.iter().find_map(|x| {
                        x.get("text")
                            .and_then(|t| t.get("sv"))
                            .and_then(Value::as_str)
                    })
                })
                .unwrap_or_default()
                .to_string();
            let start = a
                .get("approximateStart")
                .and_then(Value::as_str)
                .and_then(ts);
            let end = a.get("approximateEnd").and_then(Value::as_str).and_then(ts);
            let desc = sv(a, "eventDescription");
            out.push(Warning {
                id: format!(
                    "{}-{}",
                    w.get("id").map(Value::to_string).unwrap_or_default(),
                    a.get("id").map(Value::to_string).unwrap_or_default()
                ),
                level,
                event: if desc.is_empty() { event.clone() } else { desc },
                area: sv(a, "areaName"),
                text,
                start,
                end,
            });
        }
    }
    out.sort_by(|a, b| b.level.cmp(&a.level).then(a.start.cmp(&b.start)));
    Ok(out)
}

// ---------------------------------------------------------------- VMA

#[derive(Clone, Debug, PartialEq)]
pub struct Vma {
    pub id: String,
    pub sent: Option<i64>,
    pub event: String,
    pub text: String,
    pub area: String,
    pub web: String,
    pub test: bool,
}

pub fn parse_vma(text: &str) -> Result<Vec<Vma>, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "VMA: not JSON")?;
    let list = v
        .get("alerts")
        .and_then(Value::as_array)
        .ok_or("VMA: unexpected answer")?;
    let mut out = Vec::new();
    for a in list {
        if a.get("msgType").and_then(Value::as_str) == Some("Cancel") {
            continue;
        }
        let infos = a
            .get("info")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let info = infos
            .iter()
            .find(|i| {
                i.get("language")
                    .and_then(Value::as_str)
                    .is_some_and(|l| l.starts_with("sv"))
            })
            .or(infos.first())
            .cloned()
            .unwrap_or(Value::Null);
        let text = info
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let area = info
            .get("area")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|x| x.get("areaDesc").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        out.push(Vma {
            id: a
                .get("identifier")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            sent: a.get("sent").and_then(Value::as_str).and_then(ts),
            event: info
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or("VMA")
                .to_string(),
            test: a.get("status").and_then(Value::as_str) == Some("Test")
                || text.trim_start().starts_with("TEST"),
            text,
            area,
            web: info
                .get("web")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------- Krisinformation

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub id: String,
    pub at: i64,
    pub title: String,
    pub text: String,
    pub area: String,
    pub link: String,
    pub local: bool,
}

pub fn parse_kris(text: &str, cfg: &Config) -> Result<Vec<Notice>, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "Krisinformation: not JSON")?;
    let list = v.as_array().ok_or("Krisinformation: unexpected answer")?;
    let mut out = Vec::new();
    for n in list {
        let areas: Vec<String> = n
            .get("area")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|a| {
                        a.get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let national = areas.is_empty() || areas.iter().any(|a| a == "Sverige");
        let county = areas
            .iter()
            .any(|a| a.eq_ignore_ascii_case(&cfg.county_name) || a.contains(&cfg.municipality));
        if !national && !county {
            continue;
        }
        let title = n
            .get("headline")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let text = n
            .get("preamble")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        out.push(Notice {
            id: n
                .get("identifier")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            at: n
                .get("published")
                .and_then(Value::as_str)
                .and_then(ts)
                .unwrap_or(0),
            local: county || cfg.is_local(&title) || cfg.is_local(&text),
            area: areas.join(", "),
            link: n
                .get("web")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            title,
            text,
        });
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.at));
    Ok(out)
}

// ---------------------------------------------------------------- news

/// The text of an HTML fragment: tags dropped, entities decoded, blank
/// runs collapsed, list items and paragraphs as sentences.
pub fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in html.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let t = tag
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                if matches!(t.as_str(), "p" | "li" | "br" | "div") {
                    out.push(' ');
                }
                tag.clear();
            } else {
                tag.push(c);
            }
        } else if c == '<' {
            in_tag = true;
        } else {
            out.push(c);
        }
    }
    let out = decode_entities(&out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            n if n.starts_with("#x") => u32::from_str_radix(&n[2..], 16)
                .ok()
                .and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Story {
    pub source: usize,
    pub at: i64,
    pub title: String,
    pub text: String,
    pub link: String,
    pub local: bool,
}

/// A Sveriges Radio program feed (Atom).
pub fn parse_news(text: &str, source: usize, cfg: &Config) -> Result<Vec<Story>, String> {
    let doc = roxmltree::Document::parse(text).map_err(|_| "Sveriges Radio: not a feed")?;
    let mut out = Vec::new();
    for e in doc
        .descendants()
        .filter(|n| n.has_tag_name("entry") || n.has_tag_name("item"))
    {
        let child = |name: &str| e.children().find(|c| c.has_tag_name(name));
        let title = child("title")
            .and_then(|n| n.text())
            .map(html_text)
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let body = child("summary")
            .or_else(|| child("description"))
            .and_then(|n| n.text())
            .map(html_text)
            .unwrap_or_default();
        let at = child("published")
            .or_else(|| child("updated"))
            .and_then(|n| n.text())
            .and_then(ts)
            .or_else(|| {
                child("pubDate")
                    .and_then(|n| n.text())
                    .and_then(|t| DateTime::parse_from_rfc2822(t).ok())
                    .map(|d| d.timestamp())
            })
            .unwrap_or(0);
        let link = child("link")
            .and_then(|n| {
                n.attribute("href")
                    .map(str::to_string)
                    .or_else(|| n.text().map(str::to_string))
            })
            .unwrap_or_default();
        out.push(Story {
            source,
            at,
            local: cfg.is_local(&title) || cfg.is_local(&body),
            title,
            text: body,
            link,
        });
    }
    if out.is_empty()
        && !doc
            .descendants()
            .any(|n| n.has_tag_name("feed") || n.has_tag_name("rss"))
    {
        return Err("Sveriges Radio: not a feed".into());
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.at));
    Ok(out)
}

/// The newest broadcast of a program, for "Listen": (title, audio URL).
pub fn parse_latest_episode(text: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let e = v.get("episodes")?.as_array()?.first()?;
    let file = e
        .get("listenpodfile")
        .or_else(|| e.get("downloadpodfile"))?;
    let url = file.get("url")?.as_str()?.to_string();
    let title = file
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| e.get("title").and_then(Value::as_str))
        .unwrap_or("Sveriges Radio");
    url.starts_with("http").then(|| (title.to_string(), url))
}

// ---------------------------------------------------------------- prices

#[derive(Clone, Debug, PartialEq)]
pub struct Price {
    pub from: i64,
    pub to: i64,
    /// Öre per kWh, spot price without VAT.
    pub ore: f64,
}

pub fn parse_prices(text: &str) -> Result<Vec<Price>, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "elpriset: not JSON")?;
    let list = v.as_array().ok_or("elpriset: unexpected answer")?;
    let mut out: Vec<Price> = list
        .iter()
        .filter_map(|p| {
            Some(Price {
                from: ts(p.get("time_start")?.as_str()?)?,
                to: ts(p.get("time_end")?.as_str()?)?,
                ore: p.get("SEK_per_kWh")?.as_f64()? * 100.0,
            })
        })
        .collect();
    out.sort_by_key(|p| p.from);
    if out.is_empty() {
        return Err("elpriset: no prices".into());
    }
    Ok(out)
}

/// Hourly means of quarter-hour prices: (hour start, öre).
pub fn hourly(prices: &[Price]) -> Vec<(i64, f64)> {
    let mut out: Vec<(i64, f64, u32)> = Vec::new();
    for p in prices {
        let h = p.from - p.from.rem_euclid(3600);
        match out.last_mut() {
            Some(last) if last.0 == h => {
                last.1 += p.ore;
                last.2 += 1;
            }
            _ => out.push((h, p.ore, 1)),
        }
    }
    out.into_iter()
        .map(|(h, sum, n)| (h, sum / f64::from(n)))
        .collect()
}

/// The cheapest run of `len` consecutive hours that has not ended yet.
pub fn cheapest(hours: &[(i64, f64)], len: usize, now: i64) -> Option<(i64, f64)> {
    let future: Vec<_> = hours.iter().filter(|h| h.0 + 3600 > now).copied().collect();
    future
        .windows(len)
        .map(|w| (w[0].0, w.iter().map(|h| h.1).sum::<f64>() / len as f64))
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

// ---------------------------------------------------------------- the day

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DayInfo {
    pub names: Vec<String>,
    pub flag: String,
    pub holiday: String,
    pub red: bool,
}

pub fn parse_day(text: &str) -> Result<DayInfo, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "namnsdag: not JSON")?;
    let d = v
        .get("dagar")
        .and_then(Value::as_array)
        .and_then(|l| l.first())
        .ok_or("namnsdag: no day")?;
    Ok(DayInfo {
        names: d
            .get("namnsdag")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|n| n.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        flag: d
            .get("flaggdag")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        holiday: d
            .get("helgdag")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        red: d.get("röd dag").and_then(Value::as_str) == Some("Ja"),
    })
}

// ---------------------------------------------------------------- traffic

#[derive(Clone, Debug, PartialEq)]
pub struct Incident {
    pub id: String,
    pub header: String,
    pub kind: String,
    pub text: String,
    pub road: String,
    pub place: String,
    pub severity: String,
    pub severity_code: i64,
    pub start: Option<i64>,
    pub end: Option<i64>,
}

fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Trafikverket's query: deviations within the radius of the place.
pub fn traffic_request(cfg: &Config) -> String {
    let fields = [
        "Deviation.Id",
        "Deviation.Header",
        "Deviation.Message",
        "Deviation.MessageType",
        "Deviation.SeverityText",
        "Deviation.SeverityCode",
        "Deviation.RoadNumber",
        "Deviation.LocationDescriptor",
        "Deviation.StartTime",
        "Deviation.EndTime",
    ];
    let include: String = fields
        .iter()
        .map(|f| format!("<INCLUDE>{f}</INCLUDE>"))
        .collect();
    format!(
        "<REQUEST><LOGIN authenticationkey=\"{}\" /><QUERY objecttype=\"Situation\" schemaversion=\"1.5\" limit=\"60\"><FILTER><NEAR name=\"Deviation.Geometry.Point.WGS84\" value=\"{:.5} {:.5}\" mindistance=\"0\" maxdistance=\"{}\" /></FILTER>{include}</QUERY></REQUEST>",
        xml_attr(cfg.traffic_key.trim()),
        cfg.longitude,
        cfg.latitude,
        cfg.traffic_radius_km * 1000
    )
}

pub fn parse_traffic(text: &str) -> Result<Vec<Incident>, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "Trafikverket: not JSON")?;
    let result = v
        .get("RESPONSE")
        .and_then(|r| r.get("RESULT"))
        .and_then(Value::as_array)
        .ok_or("Trafikverket: unexpected answer")?;
    if let Some(err) = result.iter().find_map(|r| r.get("ERROR")) {
        let msg = err
            .get("MESSAGE")
            .and_then(Value::as_str)
            .unwrap_or("error");
        return Err(format!("Trafikverket: {msg}"));
    }
    let mut out = Vec::new();
    for r in result {
        for s in r
            .get("Situation")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            for d in s
                .get("Deviation")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let g = |k: &str| {
                    d.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string()
                };
                let header = g("Header");
                let kind = g("MessageType");
                if header.is_empty() && kind.is_empty() {
                    continue;
                }
                out.push(Incident {
                    id: g("Id"),
                    header,
                    kind,
                    text: g("Message"),
                    road: g("RoadNumber"),
                    place: g("LocationDescriptor"),
                    severity: g("SeverityText"),
                    severity_code: d.get("SeverityCode").and_then(Value::as_i64).unwrap_or(0),
                    start: d.get("StartTime").and_then(Value::as_str).and_then(ts),
                    end: d.get("EndTime").and_then(Value::as_str).and_then(ts),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        b.severity_code
            .cmp(&a.severity_code)
            .then(a.road.cmp(&b.road))
    });
    out.dedup_by(|a, b| !a.id.is_empty() && a.id == b.id);
    Ok(out)
}

pub fn local_time(utc: i64) -> chrono::NaiveDateTime {
    DateTime::from_timestamp(utc, 0)
        .map(|d| d.with_timezone(&Local).naive_local())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    #[test]
    fn smhi_forecast_hours_and_days() {
        let h = parse_weather(&fixture("smhi-forecast.json")).unwrap();
        assert!(h.len() > 40);
        assert!(h[0].temp.is_finite() && h[0].symbol >= 1);
        assert!(
            h.iter().any(|x| x.span > 1.0),
            "later steps are longer than an hour"
        );
        let d = days(&h, local_time);
        assert!(d.len() >= 7);
        assert!(d.iter().all(|x| x.low <= x.high));
    }

    #[test]
    fn wind_chill_only_when_cold_and_windy() {
        assert_eq!(feels_like(15.0, 8.0), 15.0);
        assert_eq!(feels_like(-3.0, 0.5), -3.0);
        assert!(feels_like(-3.0, 8.0) < -9.0);
    }

    #[test]
    fn warnings_for_the_county_only_worst_first() {
        let w = parse_warnings(&fixture("smhi-warnings.json"), 14).unwrap();
        assert!(!w.is_empty());
        assert!(w.windows(2).all(|p| p[0].level >= p[1].level));
        assert!(parse_warnings(&fixture("smhi-warnings.json"), 99)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn vma_test_messages_are_marked() {
        let v = parse_vma(&fixture("vma-14.json")).unwrap();
        assert_eq!(v.len(), 1);
        assert!(v[0].test);
        assert!(v[0].area.contains("Uddevalla"));
        assert!(parse_vma("{\"alerts\":[]}").unwrap().is_empty());
    }

    #[test]
    fn krisinformation_national_and_county_kept() {
        let cfg = Config::default();
        let n = parse_kris(&fixture("krisinformation.json"), &cfg).unwrap();
        assert!(n.iter().any(|x| x.area == "Sverige"));
        assert!(n.iter().any(|x| x.local));
        assert!(!n.iter().any(|x| x.area.contains("Skåne")));
    }

    #[test]
    fn sr_feed_stories_with_local_marks() {
        let cfg = Config::default();
        let s = parse_news(&fixture("sr-125.xml"), 0, &cfg).unwrap();
        assert!(s.len() >= 3);
        assert!(s.windows(2).all(|p| p[0].at >= p[1].at));
        assert!(s.iter().any(|x| x.local));
        assert!(s
            .iter()
            .all(|x| !x.title.contains('<') && !x.text.contains("&lt;")));
        assert!(s[0].link.starts_with("https://"));
    }

    #[test]
    fn html_to_text() {
        assert_eq!(
            html_text("<ul><li><p>Ett &amp; två</p></li><li>tre&#8230;</li></ul>"),
            "Ett & två tre…"
        );
    }

    #[test]
    fn latest_episode_audio() {
        let (title, url) = parse_latest_episode(&fixture("sr-latest-5380.json")).unwrap();
        assert!(title.starts_with("Ekot"));
        assert!(url.starts_with("https://"));
    }

    #[test]
    fn prices_quarters_to_hours_and_cheapest_run() {
        let p = parse_prices(&fixture("prices.json")).unwrap();
        let h = hourly(&p);
        assert_eq!(h.len(), 24);
        let (start, mean) = cheapest(&h, 3, 0).unwrap();
        assert!(h.iter().any(|x| x.0 == start));
        assert!(mean <= h.iter().map(|x| x.1).sum::<f64>() / 24.0);
    }

    #[test]
    fn name_day_and_flags() {
        let d = parse_day(&fixture("day.json")).unwrap();
        assert_eq!(d.names, vec!["Gudrun".to_string(), "Rune".to_string()]);
        assert!(!d.red);
    }

    #[test]
    fn traffic_request_and_answer() {
        let cfg = Config {
            traffic_key: "k\"<x>".into(),
            ..Config::default()
        };
        let q = traffic_request(&cfg);
        assert!(q.contains("authenticationkey=\"k&quot;&lt;x&gt;\""));
        assert!(q.contains("value=\"11.92110 58.22350\""));
        assert!(q.contains("maxdistance=\"30000\""));
        let t = parse_traffic(&fixture("traffic.json")).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t[0].severity_code >= t[1].severity_code);
        assert!(parse_traffic("{\"RESPONSE\":{\"RESULT\":[{\"ERROR\":{\"SOURCE\":\"x\",\"MESSAGE\":\"Invalid key\"}}]}}").unwrap_err().contains("Invalid key"));
    }
}
