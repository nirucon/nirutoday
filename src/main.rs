//! NIRUTODAY: today at a glance for NIRUX, in a teletext look. SMHI
//! weather, Sveriges Radio news (P4 Väst and Ekot by default, local news
//! first), VMA and warnings, the electricity price, the name day, your
//! calendar and, when you turn it on, traffic from Trafikverket.
mod app;
mod calendar;
mod canvas;
mod config;
mod icons;
mod ics;
mod net;
mod play;
mod sources;
mod sun;
mod sv;
mod text;
mod theme;
mod tz;
mod ui;
mod x;

use sources::Kind;
use std::process::ExitCode;

const HELP: &str = "\
nirutoday            the page (keys: ? in the window)
nirutoday --print    today as text, for a terminal (fetches what is stale)
nirutoday --version

Settings: ~/.config/nirutoday/config.toml (written with the defaults at the
first start). Cache: ~/.cache/nirutoday/.";

fn print() -> ExitCode {
    let home = theme::config_home();
    let cfg = config::load(&home);
    let now = net::now();
    let today = chrono::Local::now().date_naive();
    let mut d = app::from_cache(&cfg);
    // Refresh what is older than its interval, in parallel.
    let stale: Vec<Kind> = Kind::all(&cfg)
        .into_iter()
        .filter(|k| net::cached(&k.key(&cfg, today)).is_none_or(|(_, at)| now - at > k.interval()))
        .collect();
    let handles: Vec<_> = stale
        .into_iter()
        .map(|k| {
            let c = cfg.clone();
            std::thread::spawn(move || (k, app::fetch(k, &c)))
        })
        .collect();
    for h in handles {
        if let Ok((k, r)) = h.join() {
            match r {
                Ok(p) => app::apply(&mut d, p),
                Err(e) if k != Kind::Prices(1) => d.errors.push(format!("{k:?}: {e}")),
                Err(_) => {}
            }
        }
    }
    match app::load_calendar(&cfg) {
        Ok(e) => d.calendar = e,
        Err(e) => d.calendar_note = Some(e),
    }
    let local = sources::local_time;
    println!(
        "NIRUTODAY · {} · v. {} · {}",
        sv::date_long(today),
        sv::week(today),
        cfg.place
    );
    for a in &d.vma {
        println!(
            "VMA{} {} · {}",
            if a.test { " TEST" } else { "" },
            a.area,
            a.text
        );
    }
    if let Some(h) = d.weather.iter().find(|h| h.at > now) {
        println!(
            "Weather  {}, feels {}, wind {} m/s, rain {} mm/h",
            sv::temp(h.temp),
            sv::temp(sources::feels_like(h.temp, h.wind)),
            sv::dec1(h.wind),
            sv::dec1(h.rain)
        );
        for day in sources::days(&d.weather, local)
            .iter()
            .filter(|x| x.date >= today)
            .take(7)
        {
            println!(
                "  {:<10} {:>4} {:>4}  {} mm",
                sv::date_short(day.date),
                sv::temp(day.high),
                sv::temp(day.low),
                sv::dec1(day.rain)
            );
        }
    }
    for w in d
        .warnings
        .iter()
        .filter(|w| w.level > sources::Level::Message)
    {
        println!("Warning  {:?} {} · {}", w.level, w.event, w.area);
    }
    if let Some(day) = &d.day {
        if !day.names.is_empty() {
            println!("Name day {}", sv::and_list(&day.names));
        }
    }
    let s = sun::sun(today, cfg.latitude, cfg.longitude);
    if let (Some(a), Some(b)) = (s.rise, s.set) {
        println!(
            "Sun      {}–{} ({})",
            sv::time(local(a)),
            sv::time(local(b)),
            sv::duration(s.day_minutes)
        );
    }
    if let Some(q) = d.prices[0].iter().find(|q| q.from <= now && now < q.to) {
        println!("Power    {:.0} öre/kWh now ({})", q.ore, cfg.price_area);
    }
    for e in d.calendar.iter().take(6) {
        let when = match e.span {
            ics::Span::AllDay(..) => "all day".to_string(),
            ics::Span::Timed(a, _) => sv::time(local(a)),
        };
        println!("Calendar {when} {}", e.summary);
    }
    for st in ui::stories(&d, &cfg, 0, now).iter().take(8) {
        let src = cfg
            .news
            .get(st.source)
            .map(|n| n.name.as_str())
            .unwrap_or("");
        println!(
            "News     {} {}{} ({src})",
            sv::time(local(st.at)),
            if st.local { "● " } else { "" },
            st.title
        );
    }
    for i in d.traffic.iter().take(5) {
        println!("Traffic  {} · {} · {}", i.road, i.kind, i.place);
    }
    for e in &d.errors {
        eprintln!("nirutoday: {e}");
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version" | "version") => {
            println!("nirutoday {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some("--print" | "-p") => return print(),
        Some(other) => {
            eprintln!("nirutoday: unknown option {other} (see --help)");
            return ExitCode::FAILURE;
        }
        None => {}
    }
    match app::App::new() {
        Ok(mut a) => {
            a.run();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nirutoday: {e}");
            ExitCode::FAILURE
        }
    }
}
