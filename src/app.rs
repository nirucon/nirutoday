//! The window's loop: X events, fetched data from short-lived worker
//! threads (a pipe wakes the poll), a refresh plan per source, and a
//! redraw only when something changed or the minute turns. A hidden window
//! (another workspace, a closed scratchpad) fetches only VMA and warnings.
use crate::calendar;
use crate::canvas::Canvas;
use crate::config::{self, Config};
use crate::net;
use crate::sources::{self, Kind};
use crate::text::Text;
use crate::theme::{self, Palette};
use crate::ui::{self, Action, Data, Hit, View};
use crate::x::X;
use chrono::Local;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;

pub enum Parsed {
    Weather(Vec<sources::Hour>),
    Warnings(Vec<sources::Warning>),
    Vma(Vec<sources::Vma>),
    Kris(Vec<sources::Notice>),
    News(usize, Vec<sources::Story>),
    Prices(u8, Vec<sources::Price>),
    Day(sources::DayInfo),
    Traffic(Vec<sources::Incident>),
}

pub enum Msg {
    Got(Kind, Result<Parsed, String>),
    Calendar(Result<Vec<calendar::Entry>, String>),
    Played(Result<String, String>),
}

/// Parse an answer for `kind`.
pub fn parse(kind: Kind, text: &str, cfg: &Config) -> Result<Parsed, String> {
    Ok(match kind {
        Kind::Weather => Parsed::Weather(sources::parse_weather(text)?),
        Kind::Warnings => Parsed::Warnings(sources::parse_warnings(text, cfg.county)?),
        Kind::Vma => Parsed::Vma(sources::parse_vma(text)?),
        Kind::Kris => Parsed::Kris(sources::parse_kris(text, cfg)?),
        Kind::News(i) => Parsed::News(i, sources::parse_news(text, i, cfg)?),
        Kind::Prices(d) => Parsed::Prices(d, sources::parse_prices(text)?),
        Kind::Day => Parsed::Day(sources::parse_day(text)?),
        Kind::Traffic => Parsed::Traffic(sources::parse_traffic(text)?),
    })
}

/// Fetch and parse one source (worker thread or --print).
pub fn fetch(kind: Kind, cfg: &Config) -> Result<Parsed, String> {
    let today = Local::now().date_naive();
    let key = kind.key(cfg, today);
    let url = kind.url(cfg, today);
    let body = kind.body(cfg);
    let text = net::fetch(&net::Req {
        key: &key,
        url: &url,
        body: body.as_deref(),
    })?;
    parse(kind, &text, cfg)
}

/// Apply parsed data to the page.
pub fn apply(d: &mut Data, p: Parsed) {
    match p {
        Parsed::Weather(v) => d.weather = v,
        Parsed::Warnings(v) => d.warnings = v,
        Parsed::Vma(v) => d.vma = v,
        Parsed::Kris(v) => d.kris = v,
        Parsed::News(i, v) => {
            if d.news.len() <= i {
                d.news.resize(i + 1, Vec::new());
            }
            d.news[i] = v;
        }
        Parsed::Prices(i, v) => d.prices[usize::from(i).min(1)] = v,
        Parsed::Day(v) => d.day = Some(v),
        Parsed::Traffic(v) => {
            d.traffic = v;
            d.traffic_note = None;
        }
    }
}

/// The calendar for today and tomorrow (worker thread or --print).
pub fn load_calendar(cfg: &Config) -> Result<Vec<calendar::Entry>, String> {
    if !cfg.calendar {
        return Ok(Vec::new());
    }
    let feeds = calendar::feeds(&theme::config_home())?;
    if feeds.is_empty() {
        return Err("No calendars (add them to NIRUBAR's calendars.toml)".into());
    }
    let mut all = Vec::new();
    let mut fails = Vec::new();
    for (i, f) in feeds.iter().enumerate() {
        match calendar::events(i, f) {
            Ok(e) => all.push((f.clone(), e)),
            Err(e) => fails.push(e),
        }
    }
    let today = Local::now().date_naive();
    let entries = calendar::entries(&all, today, today + chrono::Duration::days(2));
    if all.is_empty() && !fails.is_empty() {
        return Err(fails.join("; "));
    }
    Ok(entries)
}

/// Today's calendar from the caches only (for `--brief`).
pub fn cached_calendar(cfg: &Config) -> Vec<calendar::Entry> {
    if !cfg.calendar {
        return Vec::new();
    }
    let feeds = calendar::feeds(&theme::config_home()).unwrap_or_default();
    let all: Vec<_> = feeds
        .iter()
        .enumerate()
        .filter_map(|(i, f)| calendar::events_cached(i, f).map(|e| (f.clone(), e)))
        .collect();
    let today = Local::now().date_naive();
    calendar::entries(&all, today, today + chrono::Duration::days(1))
}

/// Data from the cache, for the first paint and offline.
pub fn from_cache(cfg: &Config) -> Data {
    let mut d = Data {
        news: vec![Vec::new(); cfg.news.len()],
        ..Data::default()
    };
    let today = Local::now().date_naive();
    let mut newest = 0;
    for k in Kind::all(cfg) {
        if let Some((text, at)) = net::cached(&k.key(cfg, today)) {
            if let Ok(p) = parse(k, &text, cfg) {
                apply(&mut d, p);
                newest = newest.max(at);
            }
        }
    }
    d.updated = newest;
    d
}

struct Plan {
    next: i64,
    busy: bool,
    fails: u32,
}

pub struct App {
    x: X,
    cv: Canvas,
    text: Text,
    base_px: f32,
    pal: Palette,
    cfg: Config,
    data: Data,
    view: View,
    hits: Vec<Hit>,
    plan: HashMap<Kind, Plan>,
    calendar_next: i64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    wake_r: i32,
    wake_w: i32,
    dirty: bool,
    shown: bool,
    quit: bool,
    minute: i64,
    day: chrono::NaiveDate,
    status_until: i64,
    notified: String,
}

fn pipe() -> (i32, i32) {
    let mut fds = [0i32; 2];
    unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) };
    (fds[0], fds[1])
}

fn poke(fd: i32) {
    let b = [1u8];
    unsafe { libc::write(fd, b.as_ptr() as *const libc::c_void, 1) };
}

fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();
}

fn notify(title: &str, body: &str, critical: bool) {
    let _ = std::process::Command::new("notify-send")
        .args([
            "-a",
            "NIRUTODAY",
            "-u",
            if critical { "critical" } else { "normal" },
            title,
            body,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| std::thread::spawn(move || c.wait()));
}

impl App {
    pub fn new() -> Result<Self, String> {
        let home = theme::config_home();
        config::ensure_example(&home);
        let cfg = config::load(&home);
        let x = X::open("NIRUTODAY").map_err(|e| format!("cannot open the display: {e}"))?;
        let name = x
            .root_theme()
            .unwrap_or_else(|| theme::configured_theme(&home));
        let pal = theme::load(&home, &name);
        let text = Text::new(theme::configured_font_path(&home).as_deref())?;
        let base_px = theme::configured_font_size(&home)
            .map(|s| s * 1.15)
            .unwrap_or(15.0);
        let mut data = from_cache(&cfg);
        data.errors.extend(cfg.problems.iter().cloned());
        let (tx, rx) = channel();
        let (wake_r, wake_w) = pipe();
        let now = net::now();
        let plan = Kind::all(&cfg)
            .into_iter()
            .map(|k| {
                (
                    k,
                    Plan {
                        next: now,
                        busy: false,
                        fails: 0,
                    },
                )
            })
            .collect();
        Ok(Self {
            x,
            cv: Canvas::new(1, 1),
            text,
            base_px,
            pal,
            cfg,
            data,
            view: View::default(),
            hits: Vec::new(),
            plan,
            calendar_next: now,
            tx,
            rx,
            wake_r,
            wake_w,
            dirty: true,
            shown: true,
            quit: false,
            minute: now / 60,
            day: Local::now().date_naive(),
            status_until: 0,
            notified: net::read_state("notified"),
        })
    }

    fn status(&mut self, s: String) {
        self.view.status = Some(s);
        self.status_until = net::now() + 6;
        self.dirty = true;
    }

    fn start_fetches(&mut self) {
        let now = net::now();
        let hour = Local::now()
            .format("%H")
            .to_string()
            .parse::<u32>()
            .unwrap_or(0);
        for (k, p) in self.plan.iter_mut() {
            if p.busy || p.next > now || (!self.shown && !k.while_hidden()) {
                continue;
            }
            // Tomorrow's prices come out around 13:00.
            if *k == Kind::Prices(1) && (hour < 13 || !self.data.prices[1].is_empty()) {
                p.next = now + 900;
                continue;
            }
            p.busy = true;
            let (k, cfg, tx, w) = (*k, self.cfg.clone(), self.tx.clone(), self.wake_w);
            std::thread::spawn(move || {
                let r = fetch(k, &cfg);
                let _ = tx.send(Msg::Got(k, r));
                poke(w);
            });
        }
        if self.shown && self.cfg.calendar && self.calendar_next <= now {
            self.calendar_next = now + 900;
            let (cfg, tx, w) = (self.cfg.clone(), self.tx.clone(), self.wake_w);
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Calendar(load_calendar(&cfg)));
                poke(w);
            });
        }
    }

    fn received(&mut self, m: Msg) {
        let now = net::now();
        match m {
            Msg::Got(k, r) => {
                let interval = k.interval();
                let p = self.plan.entry(k).or_insert(Plan {
                    next: now,
                    busy: false,
                    fails: 0,
                });
                p.busy = false;
                match r {
                    Ok(parsed) => {
                        p.fails = 0;
                        p.next = now + interval;
                        apply(&mut self.data, parsed);
                        self.data.updated = now;
                        let label = format!("{k:?}");
                        self.data.errors.retain(|e| !e.starts_with(&label));
                        if matches!(k, Kind::Vma | Kind::Warnings) {
                            self.notify_new();
                        }
                    }
                    Err(e) => {
                        p.fails += 1;
                        p.next = now + (60i64 << p.fails.min(4)).min(interval);
                        // Tomorrow's prices not out yet: not an error.
                        if k != Kind::Prices(1) {
                            let label = format!("{k:?}");
                            self.data.errors.retain(|x| !x.starts_with(&label));
                            if k == Kind::Traffic {
                                self.data.traffic_note = Some(e.clone());
                            }
                            self.data.errors.push(format!("{label}: {e}"));
                        }
                    }
                }
            }
            Msg::Calendar(r) => match r {
                Ok(e) => {
                    self.data.calendar = e;
                    self.data.calendar_note = None;
                }
                Err(e) => self.data.calendar_note = Some(e),
            },
            Msg::Played(r) => match r {
                Ok(s) => self.status(s),
                Err(e) => self.status(format!("Listen: {e}")),
            },
        }
        self.dirty = true;
    }

    /// A notice for each new real VMA and orange or red warning, once.
    fn notify_new(&mut self) {
        if !self.cfg.notify {
            return;
        }
        let mut new = Vec::new();
        for a in self.data.vma.iter().filter(|a| !a.test) {
            if !self.notified.lines().any(|l| l == a.id) {
                notify(&format!("VMA · {}", a.area), &a.text, true);
                new.push(a.id.clone());
            }
        }
        for w in self
            .data
            .warnings
            .iter()
            .filter(|w| w.level >= sources::Level::Orange)
        {
            if !self.notified.lines().any(|l| l == w.id) {
                notify(
                    &format!("SMHI: {} · {}", w.event, w.area),
                    &w.text,
                    w.level == sources::Level::Red,
                );
                new.push(w.id.clone());
            }
        }
        if !new.is_empty() {
            let mut lines: Vec<&str> = self.notified.lines().collect();
            lines.extend(new.iter().map(String::as_str));
            let keep = lines.len().saturating_sub(200);
            self.notified = lines[keep..].join("\n");
            net::write_state("notified", &self.notified);
        }
    }

    fn listen(&mut self, i: usize) {
        let Some(src) = self.cfg.news.get(i).cloned() else {
            return;
        };
        let (tx, w) = (self.tx.clone(), self.wake_w);
        self.status(format!("{}: starting…", src.name));
        std::thread::spawn(move || {
            let r = if src.channel != 0 {
                crate::play::play(
                    &format!(
                        "https://www.sverigesradio.se/topsy/direkt/srapi/{}.mp3",
                        src.channel
                    ),
                    &format!("{} live", src.name),
                )
            } else {
                let key = format!("sr-latest-{}.json", src.latest);
                let url = format!(
                    "https://api.sr.se/api/v2/episodes/index?programid={}&format=json&size=1",
                    src.latest
                );
                net::fetch(&net::Req {
                    key: &key,
                    url: &url,
                    body: None,
                })
                .ok()
                .and_then(|t| sources::parse_latest_episode(&t))
                .ok_or_else(|| "the newest broadcast could not be found".to_string())
                .and_then(|(title, url)| crate::play::play(&url, &title))
            };
            let _ = tx.send(Msg::Played(r));
            poke(w);
        });
    }

    fn refresh_all(&mut self) {
        let now = net::now();
        for p in self.plan.values_mut() {
            p.next = now;
            p.fails = 0;
        }
        self.calendar_next = now;
        self.status("Refreshing…".into());
    }

    fn act(&mut self, a: Action) {
        match a {
            Action::Open(u) => open_url(&u),
            Action::Listen(i) => self.listen(i),
            Action::Tab(t) => {
                self.view.tab = t;
                self.view.selected = 0;
            }
            Action::Story(i) => {
                if self.view.selected == i {
                    self.open_selected();
                } else {
                    self.view.selected = i;
                }
            }
            Action::Refresh => self.refresh_all(),
            Action::Tomorrow => self.view.tomorrow = !self.view.tomorrow,
            Action::Help => self.view.help = !self.view.help,
        }
        self.dirty = true;
    }

    fn open_selected(&mut self) {
        let list = ui::stories(&self.data, &self.cfg, self.view.tab, net::now());
        if let Some(s) = list.get(self.view.selected) {
            open_url(&s.link.clone());
        }
    }

    fn story_count(&self) -> usize {
        ui::stories(&self.data, &self.cfg, self.view.tab, net::now()).len()
    }

    fn scroll(&mut self, by: i32) {
        self.view.scroll = (self.view.scroll + by).clamp(0, self.view.overflow.max(0));
        self.dirty = true;
    }

    fn key(&mut self, e: &KeyPressEvent) {
        let sym = self.x.keysym(e.detail, e.state);
        let tabs = ui::tab_names(&self.cfg).len();
        if self.view.help && sym != 0x3f {
            self.view.help = false;
            self.dirty = true;
            if matches!(sym, 0xff1b | 0x71) {
                return;
            }
        }
        match sym {
            0x71 | 0x51 | 0xff1b => self.quit = true,   // q Q Escape
            0x72 | 0x52 | 0xffc2 => self.refresh_all(), // r R F5
            0x3f | 0xffbe => self.view.help = !self.view.help, // ? F1
            0xff09 => {
                self.view.tab = (self.view.tab + 1) % tabs;
                self.view.selected = 0;
            }
            0xfe20 => {
                self.view.tab = (self.view.tab + tabs - 1) % tabs;
                self.view.selected = 0;
            }
            0xff54 | 0x6a => {
                self.view.selected =
                    (self.view.selected + 1).min(self.story_count().saturating_sub(1))
            }
            0xff52 | 0x6b => self.view.selected = self.view.selected.saturating_sub(1),
            0xff0d | 0xff8d => self.open_selected(),
            0x6c | 0x4c => {
                if let Some(i) = ui::listen_source(&self.cfg, self.view.tab) {
                    self.listen(i);
                }
            }
            0x74 | 0x54 => self.view.tomorrow = !self.view.tomorrow,
            0xff56 => self.scroll(self.view.rows_per_page - 2),
            0xff55 => self.scroll(-(self.view.rows_per_page - 2)),
            0xff50 => self.scroll(-10_000),
            0xff57 => self.scroll(10_000),
            0x2b | 0x3d | 0xffab => self.view.zoom = (self.view.zoom + 1).min(8),
            0x2d | 0xffad => self.view.zoom = (self.view.zoom - 1).max(-5),
            0x30 => self.view.zoom = 0,
            0x66 | 0x46 | 0xffc8 => self.x.toggle_fullscreen(),
            n @ 0x31..=0x36 => {
                // 1-6: jump down the page in one column; harmless in wide ones.
                let target = (n - 0x31) as i32 * 12;
                self.view.scroll = target.min(self.view.overflow);
            }
            _ => return,
        }
        self.dirty = true;
    }

    fn event(&mut self, ev: Event) {
        match ev {
            Event::Expose(e) if e.count == 0 => self.dirty = true,
            Event::ConfigureNotify(e) => {
                if i32::from(e.width) != self.cv.w || i32::from(e.height) != self.cv.h {
                    self.cv.resize(i32::from(e.width), i32::from(e.height));
                    self.dirty = true;
                }
            }
            Event::MapNotify(_) => {
                self.shown = true;
                self.dirty = true;
            }
            Event::UnmapNotify(_) => self.shown = false,
            Event::VisibilityNotify(e) => {
                let was = self.shown;
                self.shown = e.state != Visibility::FULLY_OBSCURED;
                if self.shown && !was {
                    self.dirty = true;
                }
            }
            Event::KeyPress(e) => self.key(&e),
            Event::ButtonPress(e) => match e.detail {
                1 => {
                    let (px, py) = (i32::from(e.event_x), i32::from(e.event_y));
                    if self.view.help {
                        self.view.help = false;
                        self.dirty = true;
                        return;
                    }
                    if let Some(h) = self.hits.iter().rev().find(|h| h.r.contains(px, py)) {
                        let a = h.a.clone();
                        self.act(a);
                    }
                }
                4 => {
                    if self.view.overflow > 0 {
                        self.scroll(-3)
                    } else {
                        self.view.selected = self.view.selected.saturating_sub(1);
                        self.dirty = true;
                    }
                }
                5 => {
                    if self.view.overflow > 0 {
                        self.scroll(3)
                    } else {
                        self.view.selected =
                            (self.view.selected + 1).min(self.story_count().saturating_sub(1));
                        self.dirty = true;
                    }
                }
                _ => {}
            },
            Event::PropertyNotify(e) if e.atom == self.x.atoms.niruwm_theme => {
                if let Some(name) = self.x.root_theme() {
                    self.pal = theme::load(&theme::config_home(), &name);
                    self.dirty = true;
                }
            }
            Event::ClientMessage(e) if e.data.as_data32()[0] == self.x.atoms.wm_delete => {
                self.quit = true
            }
            _ => {}
        }
    }

    fn tick(&mut self) {
        let now = net::now();
        if now / 60 != self.minute {
            self.minute = now / 60;
            self.dirty = true;
        }
        let today = Local::now().date_naive();
        if today != self.day {
            // A new day: new prices, name day and calendar.
            self.day = today;
            self.data.prices = [std::mem::take(&mut self.data.prices[1]), Vec::new()];
            self.data.day = None;
            for k in [Kind::Day, Kind::Prices(0), Kind::Prices(1)] {
                if let Some(p) = self.plan.get_mut(&k) {
                    p.next = now;
                }
            }
            self.calendar_next = now;
        }
        if self.view.status.is_some() && now >= self.status_until {
            self.view.status = None;
            self.dirty = true;
        }
        self.start_fetches();
    }

    fn timeout(&self) -> i32 {
        let now = net::now();
        let next_minute = 60 - now.rem_euclid(60);
        let next_fetch = self
            .plan
            .iter()
            .filter(|(k, p)| !p.busy && (self.shown || k.while_hidden()))
            .map(|(_, p)| p.next - now)
            .min()
            .unwrap_or(60);
        let mut t = next_minute.min(next_fetch.max(1));
        if self.view.status.is_some() {
            t = t.min((self.status_until - now).max(1));
        }
        (t.clamp(1, 60) * 1000) as i32
    }

    fn draw(&mut self) {
        let now = net::now();
        self.hits = ui::draw(
            &mut self.cv,
            &mut self.text,
            self.base_px,
            self.pal,
            &self.data,
            &self.cfg,
            &mut self.view,
            now,
        );
        self.view.scroll = self.view.scroll.min(self.view.overflow);
        self.x.present(&self.cv);
    }

    pub fn run(&mut self) {
        let xfd = self.x.fd();
        while !self.quit {
            while let Some(ev) = self.x.poll() {
                self.event(ev);
                if self.quit {
                    return;
                }
            }
            while let Ok(m) = self.rx.try_recv() {
                self.received(m);
            }
            self.tick();
            if self.dirty && self.shown && self.cv.w > 1 {
                self.draw();
                self.dirty = false;
            }
            self.x.flush();
            let mut fds = [
                libc::pollfd {
                    fd: xfd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.wake_r,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let timeout = self.timeout();
            unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
            if fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                return;
            }
            if fds[1].revents & libc::POLLIN != 0 {
                let mut buf = [0u8; 64];
                while unsafe {
                    libc::read(
                        self.wake_r,
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                } > 0
                {}
            }
        }
    }
}
