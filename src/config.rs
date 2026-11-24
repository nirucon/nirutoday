//! `~/.config/nirutoday/config.toml`. Every key is optional; the defaults
//! are Ljungskile in Uddevalla municipality (Västra Götaland, elområde SE3),
//! news from P4 Väst and Ekot, and traffic off. The first start writes a
//! commented copy of the defaults when the file is missing.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct NewsSource {
    pub name: String,
    /// Sveriges Radio program whose articles are listed (its Atom feed).
    pub program: u32,
    /// Channel played live by "Listen" (0: none).
    pub channel: u32,
    /// Program whose newest broadcast "Listen" plays when there is no channel.
    pub latest: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub place: String,
    pub latitude: f64,
    pub longitude: f64,
    /// County (län) number: 14 is Västra Götaland.
    pub county: u32,
    pub county_name: String,
    pub municipality: String,
    /// Places that make a news item or a message "local": shown first.
    pub nearby: Vec<String>,
    pub news: Vec<NewsSource>,
    pub news_max: usize,
    pub price_area: String,
    pub calendar: bool,
    pub traffic: bool,
    pub traffic_key: String,
    pub traffic_radius_km: u32,
    pub notify: bool,
    /// Problems found in the file, shown in the window.
    pub problems: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            place: "Ljungskile".into(),
            latitude: 58.2235,
            longitude: 11.9211,
            county: 14,
            county_name: "Västra Götalands län".into(),
            municipality: "Uddevalla".into(),
            nearby: [
                "Ljungskile",
                "Uddevalla",
                "Stenungsund",
                "Orust",
                "Lysekil",
                "Munkedal",
                "Lilla Edet",
                "Trollhättan",
                "Vänersborg",
                "Tjörn",
                "Kungälv",
                "Bohuslän",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            news: vec![
                NewsSource {
                    name: "P4 Väst".into(),
                    program: 125,
                    channel: 219,
                    latest: 0,
                },
                NewsSource {
                    name: "Ekot".into(),
                    program: 83,
                    channel: 0,
                    latest: 5380,
                },
            ],
            news_max: 30,
            price_area: "SE3".into(),
            calendar: true,
            traffic: false,
            traffic_key: String::new(),
            traffic_radius_km: 30,
            notify: true,
            problems: Vec::new(),
        }
    }
}

pub const EXAMPLE: &str = r#"# NIRUTODAY: today at a glance. Every key is optional; remove one to get
# the default back. Restart NIRUTODAY (or press R) after a change.

[place]
name = "Ljungskile"
latitude = 58.2235
longitude = 11.9211
# County (län) number for VMA, SMHI warnings and traffic: 14 is Västra Götaland.
county = 14
county_name = "Västra Götalands län"
municipality = "Uddevalla"
# News and messages that name one of these places are marked and shown first.
nearby = ["Ljungskile", "Uddevalla", "Stenungsund", "Orust", "Lysekil", "Munkedal",
          "Lilla Edet", "Trollhättan", "Vänersborg", "Tjörn", "Kungälv", "Bohuslän"]

[news]
max = 30
# Sveriges Radio: program = the news program whose articles are listed,
# channel = the channel "Listen" plays live, latest = a program whose newest
# broadcast "Listen" plays instead. See api.sr.se for other ids
# (P4 Göteborg: program 104, channel 212).
[[news.source]]
name = "P4 Väst"
program = 125
channel = 219

[[news.source]]
name = "Ekot"
program = 83
latest = 5380

[electricity]
# Elområde: SE1, SE2, SE3 or SE4.
area = "SE3"

[calendar]
# Today's events from the calendars NIRUBAR reads (~/.config/nirubar/calendars.toml).
enabled = true

[traffic]
# Trafikverket's open API needs a free key: register at
# https://data.trafikverket.se/ and put it here (keep this file private).
enabled = false
key = ""
radius_km = 30

[notify]
# A notice for a new VMA, or an orange or red SMHI warning for the county.
enabled = true
"#;

pub fn path(config_home: &Path) -> PathBuf {
    config_home.join("nirutoday/config.toml")
}

/// Write the commented example when there is no file yet.
pub fn ensure_example(config_home: &Path) {
    let p = path(config_home);
    if p.exists() {
        return;
    }
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&p, EXAMPLE);
}

pub fn load(config_home: &Path) -> Config {
    let mut c = Config::default();
    let p = path(config_home);
    let Ok(text) = std::fs::read_to_string(&p) else {
        return c;
    };
    let v = match text.parse::<toml::Value>() {
        Ok(v) => v,
        Err(e) => {
            let line = e
                .span()
                .map(|s| text[..s.start].lines().count().max(1))
                .unwrap_or(1);
            c.problems.push(format!(
                "config.toml line {line}: not valid TOML; using the defaults"
            ));
            return c;
        }
    };
    apply(&mut c, &v);
    if c.traffic && c.traffic_key.trim().is_empty() {
        c.problems
            .push("traffic is on but [traffic] key is empty".into());
    }
    if c.traffic && !private(&p) {
        c.problems
            .push("config.toml holds a key: chmod 600 ~/.config/nirutoday/config.toml".into());
    }
    c
}

fn private(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p)
        .map(|m| m.mode() & 0o077 == 0)
        .unwrap_or(true)
}

fn s(v: &toml::Value, k: &str) -> Option<String> {
    v.get(k)?.as_str().map(str::to_string)
}
fn f(v: &toml::Value, k: &str) -> Option<f64> {
    let x = v.get(k)?;
    x.as_float().or_else(|| x.as_integer().map(|i| i as f64))
}
fn u(v: &toml::Value, k: &str) -> Option<u32> {
    v.get(k)?.as_integer().and_then(|i| u32::try_from(i).ok())
}
fn b(v: &toml::Value, k: &str) -> Option<bool> {
    v.get(k)?.as_bool()
}

fn apply(c: &mut Config, v: &toml::Value) {
    if let Some(p) = v.get("place") {
        if let Some(x) = s(p, "name") {
            c.place = x;
        }
        if let Some(x) = f(p, "latitude").filter(|x| (-90.0..=90.0).contains(x)) {
            c.latitude = x;
        }
        if let Some(x) = f(p, "longitude").filter(|x| (-180.0..=180.0).contains(x)) {
            c.longitude = x;
        }
        if let Some(x) = u(p, "county").filter(|x| (1..=25).contains(x)) {
            c.county = x;
        }
        if let Some(x) = s(p, "county_name") {
            c.county_name = x;
        }
        if let Some(x) = s(p, "municipality") {
            c.municipality = x;
        }
        if let Some(list) = p.get("nearby").and_then(|x| x.as_array()) {
            c.nearby = list
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
        }
    }
    if let Some(n) = v.get("news") {
        if let Some(x) = u(n, "max") {
            c.news_max = x.clamp(5, 100) as usize;
        }
        if let Some(list) = n.get("source").and_then(|x| x.as_array()) {
            let mut out = Vec::new();
            for item in list {
                let (Some(name), Some(program)) = (s(item, "name"), u(item, "program")) else {
                    c.problems
                        .push("a [[news.source]] needs name and program".into());
                    continue;
                };
                out.push(NewsSource {
                    name,
                    program,
                    channel: u(item, "channel").unwrap_or(0),
                    latest: u(item, "latest").unwrap_or(0),
                });
            }
            if !out.is_empty() {
                c.news = out;
            }
        }
    }
    if let Some(e) = v.get("electricity") {
        if let Some(x) = s(e, "area").map(|a| a.to_uppercase()) {
            if ["SE1", "SE2", "SE3", "SE4"].contains(&x.as_str()) {
                c.price_area = x;
            } else {
                c.problems
                    .push(format!("[electricity] area {x} is not SE1-SE4"));
            }
        }
    }
    if let Some(x) = v.get("calendar").and_then(|t| b(t, "enabled")) {
        c.calendar = x;
    }
    if let Some(t) = v.get("traffic") {
        c.traffic = b(t, "enabled").unwrap_or(false);
        c.traffic_key = s(t, "key").unwrap_or_default();
        if let Some(x) = u(t, "radius_km") {
            c.traffic_radius_km = x.clamp(1, 200);
        }
    }
    if let Some(x) = v.get("notify").and_then(|t| b(t, "enabled")) {
        c.notify = x;
    }
}

impl Config {
    /// Does `text` name one of the nearby places (whole words, any case)?
    pub fn is_local(&self, text: &str) -> bool {
        let low = text.to_lowercase();
        self.nearby.iter().any(|p| {
            let p = p.to_lowercase();
            low.match_indices(&p).any(|(i, _)| {
                let before = low[..i].chars().next_back();
                let after = low[i + p.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(|c| c.is_alphabetic() && !"sn".contains(c))
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_is_the_defaults() {
        let v: toml::Value = EXAMPLE.parse().unwrap();
        let mut c = Config::default();
        apply(&mut c, &v);
        assert_eq!(c, Config::default());
    }

    #[test]
    fn local_matches_whole_place_names_and_genitive() {
        let c = Config::default();
        assert!(c.is_local("Brand i Uddevalla under natten"));
        assert!(c.is_local("Uddevallas nya simhall"));
        assert!(c.is_local("Olycka på E6 vid Ljungskile"));
        assert!(!c.is_local("Ny chef i Göteborg"));
        assert!(!c.is_local("Orustbor oroliga"));
        assert!(c.is_local("Krock på Orust"));
    }

    #[test]
    fn a_broken_file_falls_back_to_the_defaults() {
        let dir = std::env::temp_dir().join(format!("nirutoday-config-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("nirutoday")).unwrap();
        std::fs::write(path(&dir), "[place\nname = 1").unwrap();
        let c = load(&dir);
        assert_eq!(c.place, "Ljungskile");
        assert_eq!(c.problems.len(), 1);
        std::fs::write(
            path(&dir),
            "[electricity]\narea = \"se4\"\n[traffic]\nenabled = true\n",
        )
        .unwrap();
        let c = load(&dir);
        assert_eq!(c.price_area, "SE4");
        assert!(c.traffic && c.problems.iter().any(|p| p.contains("key is empty")));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
