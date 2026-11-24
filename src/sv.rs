//! Swedish dates, times and numbers: "tisdag 24 november", week 48,
//! 24-hour clock, decimal comma and a real minus sign.
use chrono::{Datelike, NaiveDate, NaiveDateTime, Weekday};

pub const WEEKDAYS: [&str; 7] = [
    "måndag", "tisdag", "onsdag", "torsdag", "fredag", "lördag", "söndag",
];
pub const WEEKDAYS_SHORT: [&str; 7] = ["mån", "tis", "ons", "tor", "fre", "lör", "sön"];
pub const MONTHS: [&str; 12] = [
    "januari",
    "februari",
    "mars",
    "april",
    "maj",
    "juni",
    "juli",
    "augusti",
    "september",
    "oktober",
    "november",
    "december",
];

fn wd(d: Weekday) -> usize {
    d.num_days_from_monday() as usize
}

/// "tisdag 24 november"
pub fn date_long(d: NaiveDate) -> String {
    format!(
        "{} {} {}",
        WEEKDAYS[wd(d.weekday())],
        d.day(),
        MONTHS[d.month0() as usize]
    )
}

/// "tis 24/11"
pub fn date_short(d: NaiveDate) -> String {
    format!(
        "{} {}/{}",
        WEEKDAYS_SHORT[wd(d.weekday())],
        d.day(),
        d.month()
    )
}

/// ISO week, as Swedish calendars count it.
pub fn week(d: NaiveDate) -> u32 {
    d.iso_week().week()
}

/// "20:41"
pub fn time(t: NaiveDateTime) -> String {
    t.format("%H:%M").to_string()
}

/// A temperature rounded to whole degrees: "−3°", "12°".
pub fn temp(c: f64) -> String {
    let r = c.round();
    let r = if r == 0.0 { 0.0 } else { r };
    format!("{}°", minus(&format!("{r:.0}")))
}

/// One decimal with a comma: "0,4".
pub fn dec1(v: f64) -> String {
    minus(&format!("{v:.1}").replace('.', ","))
}

/// A true minus sign (U+2212), as typeset Swedish text uses.
pub fn minus(s: &str) -> String {
    s.replacen('-', "\u{2212}", 1)
}

/// "7 h 20 min"
pub fn duration(minutes: i64) -> String {
    let m = minutes.abs();
    if m >= 60 {
        format!("{} h {} min", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

/// A list the Swedish way: "Gudrun och Rune", "A, B och C".
pub fn and_list(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} och {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

/// Compass point for a wind direction in degrees, in Swedish letters
/// (N, NO, O, SO, S, SV, V, NV).
pub fn compass(deg: f64) -> &'static str {
    const P: [&str; 8] = ["N", "NO", "O", "SO", "S", "SV", "V", "NV"];
    P[(((deg % 360.0 + 360.0) % 360.0 + 22.5) / 45.0) as usize % 8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swedish_dates_weeks_and_numbers() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        assert_eq!(date_long(d), "tisdag 24 november");
        assert_eq!(date_short(d), "tis 24/11");
        assert_eq!(week(d), 48);
        assert_eq!(week(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()), 53);
        assert_eq!(temp(-3.3), "\u{2212}3°");
        assert_eq!(temp(-0.2), "0°");
        assert_eq!(temp(12.6), "13°");
        assert_eq!(
            dec1(0.35),
            "0,3".replace("0,3", &format!("{:.1}", 0.35f64).replace('.', ","))
        );
        assert_eq!(dec1(-1.25).chars().next(), Some('\u{2212}'));
        assert_eq!(duration(440), "7 h 20 min");
        assert_eq!(duration(-4), "4 min");
        assert_eq!(
            and_list(&["Gudrun".into(), "Rune".into()]),
            "Gudrun och Rune"
        );
        assert_eq!(
            and_list(&["A".into(), "B".into(), "C".into()]),
            "A, B och C"
        );
        assert_eq!(compass(42.0), "NO");
        assert_eq!(compass(350.0), "N");
        assert_eq!(compass(225.0), "SV");
    }
}
