//! Sunrise and sunset from the date and the place (NOAA's simplified
//! solar position), so no service is needed. Times are UTC seconds; `None`
//! when the sun does not rise or set that day (far north in summer/winter).
use chrono::{Datelike, NaiveDate};

pub struct SunDay {
    pub rise: Option<i64>,
    pub set: Option<i64>,
    /// Minutes of daylight.
    pub day_minutes: i64,
}

pub fn sun(date: NaiveDate, lat: f64, lon: f64) -> SunDay {
    let day = date.ordinal() as f64;
    let year_days = if date.leap_year() { 366.0 } else { 365.0 };
    let g = 2.0 * std::f64::consts::PI / year_days * (day - 1.0);
    let eqtime = 229.18
        * (0.000075 + 0.001868 * g.cos()
            - 0.032077 * g.sin()
            - 0.014615 * (2.0 * g).cos()
            - 0.040849 * (2.0 * g).sin());
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos()
        + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    let phi = lat.to_radians();
    let zenith = 90.833f64.to_radians();
    let cos_ha = zenith.cos() / (phi.cos() * decl.cos()) - phi.tan() * decl.tan();
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .map(|d| d.and_utc().timestamp())
        .unwrap_or(0);
    if cos_ha > 1.0 {
        return SunDay {
            rise: None,
            set: None,
            day_minutes: 0,
        };
    }
    if cos_ha < -1.0 {
        return SunDay {
            rise: None,
            set: None,
            day_minutes: 24 * 60,
        };
    }
    let ha = cos_ha.acos().to_degrees();
    let rise_min = 720.0 - 4.0 * (lon + ha) - eqtime;
    let set_min = 720.0 - 4.0 * (lon - ha) - eqtime;
    SunDay {
        rise: Some(midnight + (rise_min * 60.0) as i64),
        set: Some(midnight + (set_min * 60.0) as i64),
        day_minutes: ((set_min - rise_min).round()) as i64,
    }
}

/// Is the sun up at UTC instant `t`?
pub fn is_day(t: i64, lat: f64, lon: f64) -> bool {
    let date = chrono::DateTime::from_timestamp(t, 0)
        .map(|d| d.date_naive())
        .unwrap_or_default();
    let s = sun(date, lat, lon);
    match (s.rise, s.set) {
        (Some(r), Some(e)) => t >= r && t < e,
        _ => s.day_minutes > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ljungskile_late_november_and_midsummer() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 24).unwrap();
        let s = sun(d, 58.2235, 11.9211);
        let local = |t: i64| {
            chrono::DateTime::from_timestamp(t + 3600, 0)
                .unwrap()
                .format("%H:%M")
                .to_string()
        };
        let (r, e) = (local(s.rise.unwrap()), local(s.set.unwrap()));
        // timeanddate: about 08:25 and 15:35 local time.
        assert!(("08:15".."08:35").contains(&r.as_str()), "{r}");
        assert!(("15:25".."15:45").contains(&e.as_str()), "{e}");
        assert!((400..460).contains(&s.day_minutes));
        let mid = sun(
            NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(),
            58.2235,
            11.9211,
        );
        assert!(mid.day_minutes > 17 * 60);
        assert_eq!(
            sun(NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(), 69.0, 20.0).day_minutes,
            24 * 60
        );
    }
}
