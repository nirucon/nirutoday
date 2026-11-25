//! Today's events from the calendars NIRUBAR reads
//! (`~/.config/nirubar/calendars.toml`, which must be private, mode 600).
//! NIRUBAR's parsed cache (`~/.cache/nirubar/calendar/`) is used when it
//! is fresh; otherwise the feed is fetched here and kept in this program's
//! own private cache. The addresses are secrets: never shown or logged.
use crate::ics::{self, Event, Occurrence, Span};
use crate::tz::Zones;
use chrono::NaiveDate;
use serde::Deserialize;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Feed {
    pub name: String,
    url: String,
    pub colour: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub span: Span,
    pub summary: String,
    pub location: String,
    pub calendar: String,
    pub colour: Option<u32>,
}

fn base(config_home: &Path) -> PathBuf {
    config_home.join("nirubar/calendars.toml")
}

/// The feeds, or why there are none.
pub fn feeds(config_home: &Path) -> Result<Vec<Feed>, String> {
    let p = base(config_home);
    let Ok(meta) = std::fs::metadata(&p) else {
        return Ok(Vec::new());
    };
    // SAFETY: getuid has no preconditions.
    if meta.uid() != unsafe { libc::getuid() } || meta.mode() & 0o077 != 0 {
        return Err(
            "calendars.toml is not private (chmod 600 ~/.config/nirubar/calendars.toml)".into(),
        );
    }
    let text =
        std::fs::read_to_string(&p).map_err(|_| "calendars.toml cannot be read".to_string())?;
    let v: toml::Value = text
        .parse()
        .map_err(|_| "calendars.toml is not valid TOML".to_string())?;
    let mut out = Vec::new();
    for c in v
        .get("calendar")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(url)) = (
            c.get("name").and_then(|x| x.as_str()),
            c.get("url").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        let url = url.trim();
        let url = url
            .strip_prefix("webcal://")
            .map(|r| format!("https://{r}"))
            .unwrap_or_else(|| url.to_string());
        if !url.starts_with("https://")
            && !url.starts_with("http://127.0.0.1")
            && !url.starts_with("http://localhost")
        {
            continue;
        }
        let colour = c
            .get("colour")
            .or_else(|| c.get("color"))
            .and_then(|x| x.as_str())
            .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches('#'), 16).ok());
        out.push(Feed {
            name: name.to_string(),
            url,
            colour,
        });
    }
    Ok(out)
}

/// NIRUBAR's cache key for an address (FNV-1a), so a cache made for another
/// address is never used.
fn fingerprint(url: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn bar_cache_file(index: usize, feed: &Feed) -> PathBuf {
    let safe: String = feed
        .name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(24)
        .collect();
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::theme::home().join(".cache"))
        .join("nirubar/calendar")
        .join(format!("{index}-{safe}.json"))
}

#[derive(Deserialize)]
struct BarCache {
    schema: u32,
    fetched: u64,
    key: String,
    events: Vec<Event>,
}

/// NIRUBAR's events for this feed when they are younger than `max_age` s.
fn from_bar(index: usize, feed: &Feed, max_age: i64) -> Option<Vec<Event>> {
    let text = std::fs::read_to_string(bar_cache_file(index, feed)).ok()?;
    let c: BarCache = serde_json::from_str(&text).ok()?;
    (c.schema == 1
        && c.key == fingerprint(&feed.url)
        && crate::net::now() - c.fetched as i64 <= max_age)
        .then_some(c.events)
}

/// Events of one feed: NIRUBAR's cache, else a fetch here (ICS kept in this
/// program's private cache), else the last ICS we kept.
pub fn events(index: usize, feed: &Feed) -> Result<Vec<Event>, String> {
    if let Some(e) = from_bar(index, feed, 3 * 3600) {
        return Ok(e);
    }
    let key = format!("calendar-{index}-{}.ics", fingerprint(&feed.url));
    match crate::net::fetch(&crate::net::Req {
        key: &key,
        url: &feed.url,
        body: None,
    }) {
        Ok(text) => Ok(ics::parse(&text)),
        Err(_) => {
            if let Some(e) = from_bar(index, feed, i64::MAX) {
                return Ok(e);
            }
            crate::net::cached(&key)
                .map(|(t, _)| ics::parse(&t))
                .ok_or_else(|| format!("calendar {} could not be fetched", feed.name))
        }
    }
}

/// A feed's events from what is kept here or by NIRUBAR, never fetched.
pub fn events_cached(index: usize, feed: &Feed) -> Option<Vec<Event>> {
    if let Some(e) = from_bar(index, feed, i64::MAX) {
        return Some(e);
    }
    let key = format!("calendar-{index}-{}.ics", fingerprint(&feed.url));
    crate::net::cached(&key).map(|(t, _)| ics::parse(&t))
}

/// The occurrences of every feed on `[from, to)`, in time order.
pub fn entries(all: &[(Feed, Vec<Event>)], from: NaiveDate, to: NaiveDate) -> Vec<Entry> {
    let mut zones = Zones::system();
    let mut out: Vec<(i64, Entry)> = Vec::new();
    for (i, (feed, events)) in all.iter().enumerate() {
        let occ: Vec<Occurrence> = ics::expand(events, i, from, to, &mut zones);
        for o in occ {
            out.push((
                o.span.sort_key(&zones),
                Entry {
                    span: o.span,
                    summary: o.summary,
                    location: o.location,
                    calendar: feed.name.clone(),
                    colour: feed.colour,
                },
            ));
        }
    }
    out.sort_by_key(|e| e.0);
    out.into_iter().map(|e| e.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_must_be_private_and_today_is_expanded() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("nirutoday-cal-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("nirubar")).unwrap();
        let p = dir.join("nirubar/calendars.toml");
        std::fs::write(&p, "[[calendar]]\nname = \"Work\"\nurl = \"webcal://example.invalid/a.ics\"\ncolour = \"#88c0d0\"\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(feeds(&dir).is_err());
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        let f = feeds(&dir).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].url, "https://example.invalid/a.ics");
        assert_eq!(f[0].colour, Some(0x88c0d0));
        let ics_text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:1\r\nSUMMARY:Standup\r\nDTSTART:20261124T080000Z\r\nDTEND:20261124T081500Z\r\nRRULE:FREQ=DAILY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let ev = ics::parse(ics_text);
        let d = NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        let e = entries(&[(f[0].clone(), ev)], d, d.succ_opt().unwrap());
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].summary, "Standup");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
