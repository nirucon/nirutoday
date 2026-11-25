//! The page, in a teletext look: one fixed character grid, block colours,
//! mosaic weather pictures and bar charts made of blocks. The layout
//! follows the window: three columns on a wide screen, two on a laptop,
//! one in a narrow window or a scratchpad, scrolling when it is too short.
use crate::calendar::Entry;
use crate::canvas::{Canvas, Rect};
use crate::config::Config;
use crate::icons;
use crate::ics::Span;
use crate::sources::{self, DayInfo, Hour, Incident, Level, Notice, Price, Story, Vma, Warning};
use crate::sv;
use crate::text::{Size, Text};
use crate::theme::{blend, Palette};
use chrono::{Local, NaiveDate, NaiveDateTime, Timelike};

/// Everything the page shows.
#[derive(Default)]
pub struct Data {
    pub weather: Vec<Hour>,
    pub warnings: Vec<Warning>,
    pub vma: Vec<Vma>,
    pub kris: Vec<Notice>,
    pub news: Vec<Vec<Story>>,
    pub prices: [Vec<Price>; 2],
    pub day: Option<DayInfo>,
    pub calendar: Vec<Entry>,
    pub calendar_note: Option<String>,
    pub traffic: Vec<Incident>,
    pub traffic_note: Option<String>,
    /// Problems, newest last, shown in the footer.
    pub errors: Vec<String>,
    /// When the data was last refreshed (UTC seconds).
    pub updated: i64,
}

/// The reader's view: what is selected, scrolled and open.
#[derive(Default)]
pub struct View {
    /// 0 all, 1 local, then one per news source.
    pub tab: usize,
    pub selected: usize,
    pub scroll: i32,
    pub zoom: i32,
    pub help: bool,
    pub tomorrow: bool,
    pub status: Option<String>,
    /// Rows of content beyond the window (for scrolling), set by draw.
    pub overflow: i32,
    pub rows_per_page: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open(String),
    Listen(usize),
    Tab(usize),
    Story(usize),
    Refresh,
    Tomorrow,
    Help,
}

pub struct Hit {
    pub r: Rect,
    pub a: Action,
}

/// One story in the order the news panel lists it.
pub fn stories<'a>(d: &'a Data, cfg: &Config, tab: usize, now: i64) -> Vec<&'a Story> {
    let mut all: Vec<&Story> = match tab {
        0 | 1 => d.news.iter().flatten().collect(),
        n => d
            .news
            .get(n - 2)
            .map(|v| v.iter().collect())
            .unwrap_or_default(),
    };
    if tab == 1 {
        all.retain(|s| s.local);
    }
    all.sort_by_key(|s| std::cmp::Reverse(s.at));
    // The same story in two programs is listed once.
    let mut seen = std::collections::HashSet::new();
    all.retain(|s| seen.insert(s.title.to_lowercase()));
    if tab == 0 {
        // Local news of the last 12 hours first: what happens near home.
        let (mut near, rest): (Vec<_>, Vec<_>) = all
            .into_iter()
            .partition(|s| s.local && now - s.at < 12 * 3600);
        near.extend(rest);
        all = near;
    }
    all.truncate(cfg.news_max);
    all
}

pub fn tab_names(cfg: &Config) -> Vec<String> {
    let mut v = vec!["ALL".to_string(), "LOCAL".to_string()];
    v.extend(cfg.news.iter().map(|n| n.name.to_uppercase()));
    v
}

struct P<'a> {
    cv: &'a mut Canvas,
    t: &'a mut Text,
    s: Size,
    big: Size,
    huge: Size,
    cw: i32,
    ch: i32,
    pal: Palette,
    hits: Vec<Hit>,
    /// Content offset (scrolling) and the visible body.
    dy: i32,
    body: Rect,
}

fn local(utc: i64) -> NaiveDateTime {
    sources::local_time(utc)
}

impl P<'_> {
    fn y(&self, row: i32) -> i32 {
        self.body.y + row * self.ch - self.dy
    }
    fn visible(&self, row: i32, rows: i32) -> bool {
        let y = self.y(row);
        y + rows * self.ch > self.body.y && y < self.body.y + self.body.h
    }
    /// Text at column `col` (absolute) and content row `row`.
    fn put(&mut self, col: i32, row: i32, s: &str, color: u32) -> i32 {
        if !self.visible(row, 1) {
            return s.chars().count() as i32;
        }
        let y = self.y(row) + (self.ch - self.s.height) / 2;
        self.t
            .draw_grid(self.cv, col * self.cw, y, s, self.s, self.cw, color);
        s.chars().count() as i32
    }
    /// Text cut to `max` cells with "…".
    fn put_max(&mut self, col: i32, row: i32, s: &str, max: i32, color: u32) -> i32 {
        let n = s.chars().count() as i32;
        if n <= max {
            return self.put(col, row, s, color);
        }
        if max <= 1 {
            return 0;
        }
        let cut: String = s.chars().take((max - 1) as usize).collect::<String>() + "…";
        self.put(col, row, &cut, color)
    }
    fn cells(&self, col: i32, row: i32, w: i32, h: i32) -> Rect {
        Rect::new(col * self.cw, self.y(row), w * self.cw, h * self.ch)
    }
    fn fill(&mut self, col: i32, row: i32, w: i32, h: i32, color: u32) {
        let r = self.cells(col, row, w, h).intersect(&self.body);
        self.cv.fill(r, color);
    }
    fn hit(&mut self, col: i32, row: i32, w: i32, h: i32, a: Action) {
        let r = self.cells(col, row, w, h).intersect(&self.body);
        if r.w > 0 && r.h > 0 {
            self.hits.push(Hit { r, a });
        }
    }
    /// A teletext title bar: page number, title, a note on the right.
    fn title(&mut self, col: i32, row: i32, w: i32, page: u32, name: &str, note: &str) {
        let band = blend(self.pal.background, self.pal.mark(), 0.30);
        self.fill(col, row, w, 1, band);
        self.fill(col, row, 4, 1, self.pal.mark());
        self.put(col, row, &format!("{page:>3}"), self.pal.background);
        self.put(col + 5, row, name, self.pal.foreground);
        let n = note.chars().count() as i32;
        let room = w - 6 - name.chars().count() as i32 - 2;
        if n > 0 && room > 4 {
            let shown = n.min(room);
            self.put_max(col + w - shown - 1, row, note, shown, self.pal.muted);
        }
    }
    /// A small coloured block with text: VMA, RED, ORANGE...
    fn badge(&mut self, col: i32, row: i32, text: &str, bg: u32) -> i32 {
        let w = text.chars().count() as i32 + 2;
        self.fill(col, row, w, 1, bg);
        let fg = if luminance(bg) > 0.55 {
            0x101010
        } else {
            0xffffff
        };
        self.put(col + 1, row, text, fg);
        w
    }
    fn big_text(&mut self, x: i32, row: i32, s: &str, size: Size, color: u32) -> i32 {
        let y = self.y(row);
        if y + size.height < self.body.y || y > self.body.y + self.body.h {
            return self.t.width(s, size);
        }
        self.t.draw(self.cv, x, y, s, size, color)
    }
}

fn luminance(c: u32) -> f32 {
    let r = ((c >> 16) & 0xff) as f32 / 255.0;
    let g = ((c >> 8) & 0xff) as f32 / 255.0;
    let b = (c & 0xff) as f32 / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Wrap `s` to lines of at most `w` cells.
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        let need = line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
        if need > w && !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        while line.chars().count() > w {
            let head: String = line.chars().take(w).collect();
            line = line.chars().skip(w).collect();
            out.push(head);
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

pub struct Layout {
    pub cw: i32,
    pub ch: i32,
    pub px: f32,
}

/// Cell size: the suite font size, grown on tall screens so the page
/// reads from a distance (teletext fills the screen), times the zoom.
pub fn layout(t: &mut Text, base_px: f32, win_h: i32, zoom: i32) -> Layout {
    let auto = (win_h as f32 / 52.0).clamp(base_px, base_px * 1.7);
    let px = (auto * 1.1f32.powi(zoom)).clamp(9.0, 64.0).round();
    let s = t.size(px);
    let cw = (t.advance('M', s).round() as i32).max(5);
    let ch = (s.height as f32 * 1.12).round() as i32;
    Layout { cw, ch, px }
}

fn price_colour(p: &Palette, ore: f64, lo: f64, hi: f64) -> u32 {
    let third = (hi - lo) / 3.0;
    if ore <= lo + third {
        p.success
    } else if ore <= lo + 2.0 * third {
        p.warning
    } else {
        p.error
    }
}

fn level_colour(p: &Palette, l: Level) -> (u32, &'static str) {
    match l {
        Level::Red => (p.error, "RED"),
        Level::Orange => (blend(p.warning, p.error, 0.5), "ORANGE"),
        Level::Yellow => (p.warning, "YELLOW"),
        Level::Message => (blend(p.background, p.muted, 0.5), "INFO"),
    }
}

/// Draw the page. Returns the clickable areas.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    cv: &mut Canvas,
    t: &mut Text,
    base_px: f32,
    pal: Palette,
    d: &Data,
    cfg: &Config,
    v: &mut View,
    now: i64,
) -> Vec<Hit> {
    let lay = layout(t, base_px, cv.h, v.zoom);
    let s = t.size(lay.px);
    let big = t.size((lay.px * 1.9).round());
    let huge = t.size((lay.px * 3.2).round());
    cv.fill(Rect::new(0, 0, cv.w, cv.h), pal.background);
    let (cw, ch) = (lay.cw, lay.ch);
    let cols = (cv.w / cw).max(20);
    let header_rows = 2;
    let vma: Vec<&Vma> = d.vma.iter().collect();
    let banner = i32::from(!vma.is_empty());
    let body = Rect::new(
        0,
        (header_rows + banner) * ch,
        cv.w,
        cv.h - (header_rows + banner + 1) * ch,
    );
    let mut p = P {
        cv,
        t,
        s,
        big,
        huge,
        cw,
        ch,
        pal,
        hits: Vec::new(),
        dy: v.scroll * ch,
        body,
    };

    // ------------------------------------------------------------ header
    let full = Rect::new(0, 0, p.cv.w, p.cv.h);
    let today = Local::now().naive_local();
    {
        let hdr = Rect::new(0, 0, p.cv.w, header_rows * ch);
        p.cv.fill(hdr, blend(pal.background, pal.mark(), 0.16));
        p.cv.fill(
            Rect::new(
                0,
                header_rows * ch - (ch / 8).max(2),
                p.cv.w,
                (ch / 8).max(2),
            ),
            pal.mark(),
        );
        let y = (header_rows * ch - big.height) / 2;
        let x = cw;
        let pw = p.t.width("P100", big);
        p.cv.fill(Rect::new(x - cw / 2, y, pw + cw, big.height), pal.mark());
        p.t.draw(p.cv, x, y, "P100", big, pal.background);
        let after = x + pw + cw * 3 / 2;
        let w = p.t.draw(p.cv, after, y, "NIRUTODAY", big, pal.foreground);
        let clock = sv::time(today);
        let date = format!(
            "{} · v. {}",
            sv::date_long(today.date()),
            sv::week(today.date())
        );
        let cwid = p.t.width(&clock, big);
        p.t.draw(p.cv, p.cv.w - cwid - cw, y, &clock, big, pal.warning);
        let dw = p.t.width(&date, s);
        let room = p.cv.w - cwid - 3 * cw - (after + w + 2 * cw);
        if room > dw {
            let dx = after + w + 2 * cw + (room - dw) / 2;
            p.t.draw(
                p.cv,
                dx.max(after + w + 2 * cw),
                (header_rows * ch - s.height) / 2,
                &date,
                s,
                pal.foreground,
            );
        } else if room > p.t.width(&sv::date_short(today.date()), s) {
            let ds = sv::date_short(today.date());
            p.t.draw(
                p.cv,
                after + w + 2 * cw,
                (header_rows * ch - s.height) / 2,
                &ds,
                s,
                pal.foreground,
            );
        }
    }
    if let Some(a) = vma.first() {
        let r = Rect::new(0, header_rows * ch, p.cv.w, ch);
        let bg = if a.test {
            blend(pal.error, pal.background, 0.55)
        } else {
            pal.error
        };
        p.cv.fill(r, bg);
        let label = if a.test { " VMA TEST " } else { " VMA " };
        let text = format!("{label}{} · {}", a.area, a.text);
        let max = (cols - 1) as usize;
        let shown: String = if text.chars().count() > max {
            text.chars().take(max - 1).collect::<String>() + "…"
        } else {
            text
        };
        p.t.draw_grid(
            p.cv,
            0,
            header_rows * ch + (ch - s.height) / 2,
            &shown,
            s,
            cw,
            0xffffff,
        );
        if !a.web.is_empty() {
            p.hits.push(Hit {
                r,
                a: Action::Open(a.web.clone()),
            });
        }
    }

    // ------------------------------------------------------------ columns
    let ncol = if cols >= 150 {
        3
    } else if cols >= 92 {
        2
    } else {
        1
    };
    let gap = 2;
    let colw = (cols - 2 - (ncol - 1) * gap) / ncol;
    let x0 = |i: i32| 1 + i * (colw + gap);
    let page_rows = (body.h / ch).max(1);
    v.rows_per_page = page_rows;
    let mut bottoms = Vec::new();
    let weather: PanelFn = panel_weather;
    let news: PanelFn = panel_news;
    let alerts: PanelFn = panel_alerts;
    let power: PanelFn = panel_power;
    let todayp: PanelFn = panel_today;
    let traffic: PanelFn = panel_traffic;
    let mut columns: Vec<Vec<(PanelFn, bool)>> = match ncol {
        3 => vec![
            vec![(weather, false)],
            vec![(news, true)],
            vec![
                (alerts, false),
                (todayp, false),
                (power, false),
                (traffic, false),
            ],
        ],
        2 => vec![
            vec![(alerts, false), (weather, false), (power, false)],
            vec![(todayp, false), (news, true), (traffic, false)],
        ],
        _ => vec![vec![
            (alerts, false),
            (weather, false),
            (todayp, false),
            (news, true),
            (power, false),
            (traffic, false),
        ]],
    };
    if !cfg.traffic {
        for c in &mut columns {
            c.retain(|(f, _)| *f as usize != traffic as usize);
        }
    }
    for (i, col) in columns.iter().enumerate() {
        let mut row = 0;
        // The flexible panel takes what the others leave, at least 12 rows.
        let fixed: i32 = col
            .iter()
            .filter(|(_, flex)| !flex)
            .map(|(f, _)| measure(*f, colw, d, cfg, v, now))
            .sum::<i32>()
            + col.len() as i32;
        for (f, flex) in col {
            let avail = if *flex {
                (page_rows - fixed).max(14)
            } else {
                0
            };
            let used = f(&mut p, x0(i as i32), row, colw, avail, d, cfg, v, now);
            row += used + 1;
        }
        bottoms.push(row);
    }
    let content = bottoms.into_iter().max().unwrap_or(0);
    v.overflow = (content - page_rows).max(0);

    // ------------------------------------------------------------ footer
    let fy = p.cv.h - ch;
    p.cv.fill(
        Rect::new(0, fy, p.cv.w, ch),
        blend(pal.background, pal.mark(), 0.16),
    );
    let keys = if cols >= 110 {
        " R refresh  ↑↓ news  Enter open  L listen  Tab source  T tomorrow  ? keys  Q quit"
    } else {
        " R  ↑↓  Enter  L listen  ? keys"
    };
    p.t.draw_grid(p.cv, 0, fy + (ch - s.height) / 2, keys, s, cw, pal.muted);
    let status = if let Some(st) = &v.status {
        (st.clone(), pal.warning)
    } else if let Some(e) = d.errors.last() {
        (format!("offline? {e}"), pal.warning)
    } else if d.updated > 0 {
        (format!("updated {}", sv::time(local(d.updated))), pal.muted)
    } else {
        ("loading…".to_string(), pal.muted)
    };
    let used = keys.chars().count() as i32 + 2;
    let room = cols - used - 1;
    if room > 6 {
        let text: String = if status.0.chars().count() as i32 > room {
            status
                .0
                .chars()
                .take((room - 1) as usize)
                .collect::<String>()
                + "…"
        } else {
            status.0
        };
        let x = (cols - text.chars().count() as i32 - 1) * cw;
        p.t.draw_grid(p.cv, x, fy + (ch - s.height) / 2, &text, s, cw, status.1);
    }
    p.hits.push(Hit {
        r: Rect::new(0, fy, 10 * cw, ch),
        a: Action::Refresh,
    });
    if let Some(q) = keys.find("? keys") {
        let col = keys[..q].chars().count() as i32;
        p.hits.push(Hit {
            r: Rect::new(col * cw, fy, 6 * cw, ch),
            a: Action::Help,
        });
    }

    if v.help {
        help(&mut p, full, cols);
    }
    let _ = p.huge;
    p.hits
}

type PanelFn = fn(&mut P, i32, i32, i32, i32, &Data, &Config, &View, i64) -> i32;

fn measure(f: PanelFn, w: i32, d: &Data, cfg: &Config, v: &View, now: i64) -> i32 {
    // Rows a panel needs: drawn off-screen into a 1x1 canvas.
    let mut cv = Canvas::new(1, 1);
    let mut t = MEASURE
        .with(|m| m.borrow_mut().take())
        .unwrap_or_else(|| Text::new(None).expect("font"));
    let s = t.size(12.0);
    let mut p = P {
        cv: &mut cv,
        t: &mut t,
        s,
        big: s,
        huge: s,
        cw: 7,
        ch: 14,
        pal: Palette::default(),
        hits: Vec::new(),
        dy: 1 << 28,
        body: Rect::new(0, 0, 1, 1),
    };
    let rows = f(&mut p, 0, 0, w, 0, d, cfg, v, now);
    MEASURE.with(|m| *m.borrow_mut() = Some(t));
    rows
}

thread_local! {
    static MEASURE: std::cell::RefCell<Option<Text>> = const { std::cell::RefCell::new(None) };
}

// ---------------------------------------------------------------- weather

#[allow(clippy::too_many_arguments)]
fn panel_weather(
    p: &mut P,
    x: i32,
    row: i32,
    w: i32,
    _avail: i32,
    d: &Data,
    cfg: &Config,
    _v: &View,
    now: i64,
) -> i32 {
    p.title(x, row, w, 101, "WEATHER", &format!("SMHI · {}", cfg.place));
    let mut r = row + 1;
    let Some(cur) = d.weather.iter().find(|h| h.at > now).or(d.weather.last()) else {
        p.put(x + 1, r + 1, "Waiting for SMHI…", p.pal.muted);
        return 3;
    };
    let day = crate::sun::is_day(cur.at - 1800, cfg.latitude, cfg.longitude);
    // Now: a big picture, the temperature, and the details beside it.
    let icon_cols = 16.min(w / 3);
    let icon_rows = 6;
    if p.visible(r, icon_rows) {
        let ir = p.cells(x, r, icon_cols, icon_rows);
        icons::draw(p.cv, ir, cur.symbol, day, &p.pal);
    }
    let tx = (x + icon_cols + 1) * p.cw;
    let tw = p.big_text(tx, r, &sv::temp(cur.temp), p.huge, p.pal.foreground);
    let feel = sources::feels_like(cur.temp, cur.wind);
    let col = x + icon_cols + 1 + (tw / p.cw) + 2;
    let right = w - (col - x) - 1;
    if right >= 14 {
        let mut lines = vec![
            (symbol_name(cur.symbol).to_string(), p.pal.foreground),
            (format!("feels {}", sv::temp(feel)), p.pal.muted),
            (
                format!(
                    "wind {} m/s {}",
                    sv::dec1(cur.wind).trim_end_matches(",0"),
                    sv::compass(cur.wind_dir)
                ),
                p.pal.muted,
            ),
        ];
        if cur.gust.is_finite() && cur.gust > cur.wind + 2.0 {
            lines.push((
                format!("gusts {} m/s", sv::dec1(cur.gust).trim_end_matches(",0")),
                p.pal.muted,
            ));
        }
        lines.push((format!("rain {} mm/h", sv::dec1(cur.rain)), p.pal.muted));
        for (i, (l, c)) in lines.iter().take(icon_rows as usize).enumerate() {
            p.put_max(col, r + i as i32, l, right, *c);
        }
    } else {
        p.put_max(
            x + icon_cols + 1,
            r + 4,
            symbol_name(cur.symbol),
            w - icon_cols - 2,
            p.pal.muted,
        );
    }
    r += icon_rows + 1;

    // Next hours: one column of four cells per hour.
    p.put(x, r, "NEXT HOURS", p.pal.mark());
    r += 1;
    let per = 4;
    let n = ((w - 1) / per).min(24) as usize;
    let hours: Vec<&Hour> = d
        .weather
        .iter()
        .filter(|h| h.at > now && h.span <= 1.5)
        .take(n.max(1))
        .collect();
    let max_rain = hours.iter().map(|h| h.rain).fold(1.0f64, f64::max);
    for (i, h) in hours.iter().enumerate() {
        let c = x + i as i32 * per;
        let lt = local(h.at - 3600);
        p.put(c, r, &format!("{:02}", lt.hour()), p.pal.muted);
        if p.visible(r + 1, 2) {
            let ir = p.cells(c, r + 1, per - 1, 2);
            icons::draw(
                p.cv,
                ir,
                h.symbol,
                crate::sun::is_day(h.at - 1800, cfg.latitude, cfg.longitude),
                &p.pal,
            );
        }
        let tc = if h.temp < 0.0 {
            blend(p.pal.foreground, 0x5aa9ff, 0.45)
        } else {
            p.pal.foreground
        };
        p.put(c, r + 3, &sv::temp(h.temp), tc);
        if h.rain > 0.05 && p.visible(r + 4, 1) {
            let full = p.cells(c, r + 4, per - 1, 1);
            let hgt = ((h.rain / max_rain).sqrt() * full.h as f64)
                .round()
                .max(2.0) as i32;
            let bar = Rect::new(full.x, full.y + full.h - hgt, full.w, hgt).intersect(&p.body);
            p.cv.fill(bar, if p.pal.is_light() { 0x1f6fd0 } else { 0x5aa9ff });
        }
    }
    r += 6;

    // The week: two rows per day with a temperature range bar.
    p.put(x, r, "THE WEEK", p.pal.mark());
    r += 1;
    let days = sources::days(&d.weather, local);
    let days: Vec<_> = days
        .into_iter()
        .filter(|dd| dd.date >= local(now).date())
        .take(9)
        .collect();
    let lo = days.iter().map(|d| d.low).fold(f64::INFINITY, f64::min);
    let hi = days
        .iter()
        .map(|d| d.high)
        .fold(f64::NEG_INFINITY, f64::max);
    // Leave room after the bar for "27,6 mm  12 m/s".
    let bar_w = (w - 26 - 17).clamp(0, 30);
    for (i, dd) in days.iter().enumerate() {
        let rr = r + i as i32 * 2;
        let name = if i == 0 {
            "today".to_string()
        } else {
            sv::date_short(dd.date)
        };
        let red = dd.date.format("%u").to_string() == "7";
        p.put(
            x,
            rr,
            &name,
            if red { p.pal.error } else { p.pal.foreground },
        );
        if p.visible(rr, 2) {
            let ir = p.cells(x + 10, rr, 4, 2);
            icons::draw(p.cv, ir, dd.symbol, true, &p.pal);
        }
        p.put(
            x + 15,
            rr,
            &format!("{:>4}", sv::temp(dd.high)),
            p.pal.foreground,
        );
        p.put(x + 20, rr, &format!("{:>4}", sv::temp(dd.low)), p.pal.muted);
        if bar_w >= 6 && hi > lo && p.visible(rr, 1) {
            let full = p.cells(x + 26, rr, bar_w, 1);
            let px = |t: f64| full.x + ((t - lo) / (hi - lo) * full.w as f64) as i32;
            let (a, b) = (px(dd.low), px(dd.high).max(px(dd.low) + 3));
            let y = full.y + full.h / 3;
            p.cv.fill(
                Rect::new(full.x, y + full.h / 6, full.w, (full.h / 10).max(1)).intersect(&p.body),
                p.pal.border,
            );
            let cold = blend(0x5aa9ff, p.pal.foreground, 0.2);
            let warm = blend(p.pal.warning, p.pal.foreground, 0.2);
            p.cv.fill(
                Rect::new(a, y, b - a, full.h / 3).intersect(&p.body),
                if dd.high <= 0.0 { cold } else { warm },
            );
        }
        let info = format!(
            "{} mm  {} m/s",
            sv::dec1(dd.rain),
            sv::dec1(dd.wind).trim_end_matches(",0")
        );
        let ix = if bar_w >= 6 {
            x + 26 + bar_w + 1
        } else {
            x + 26
        };
        if w - (ix - x) > 6 {
            p.put_max(ix, rr, &info, w - (ix - x), p.pal.muted);
        } else {
            p.put_max(x + 15, rr + 1, &info, w - 16, p.pal.muted);
        }
    }
    r + days.len() as i32 * 2 - row
}

pub fn symbol_name(s: u8) -> &'static str {
    match s {
        1 => "Clear sky",
        2 => "Nearly clear",
        3 => "Variable clouds",
        4 => "Halfclear",
        5 => "Cloudy",
        6 => "Overcast",
        7 => "Fog",
        8 => "Light rain showers",
        9 => "Rain showers",
        10 => "Heavy rain showers",
        11 => "Thunderstorm",
        12 => "Light sleet showers",
        13 => "Sleet showers",
        14 => "Heavy sleet showers",
        15 => "Light snow showers",
        16 => "Snow showers",
        17 => "Heavy snow showers",
        18 => "Light rain",
        19 => "Rain",
        20 => "Heavy rain",
        21 => "Thunder",
        22 => "Light sleet",
        23 => "Sleet",
        24 => "Heavy sleet",
        25 => "Light snow",
        26 => "Snow",
        27 => "Heavy snow",
        _ => "",
    }
}

// ---------------------------------------------------------------- news

#[allow(clippy::too_many_arguments)]
fn panel_news(
    p: &mut P,
    x: i32,
    row: i32,
    w: i32,
    avail: i32,
    d: &Data,
    cfg: &Config,
    v: &View,
    now: i64,
) -> i32 {
    p.title(x, row, w, 102, "NEWS", "Sveriges Radio");
    let mut r = row + 1;
    // Tabs, and Listen on the right.
    let mut c = x;
    for (i, name) in tab_names(cfg).iter().enumerate() {
        let wl = name.chars().count() as i32 + 2;
        if c + wl > x + w - 10 {
            break;
        }
        if i == v.tab {
            p.fill(c, r, wl, 1, p.pal.selected);
            p.put(c + 1, r, name, p.pal.foreground);
        } else {
            p.put(c + 1, r, name, p.pal.muted);
        }
        p.hit(c, r, wl, 1, Action::Tab(i));
        c += wl + 1;
    }
    let src = listen_source(cfg, v.tab);
    if let Some(si) = src {
        let label = if cfg.news[si].channel != 0 {
            "▶ LIVE"
        } else {
            "▶ LATEST"
        };
        let lw = label.chars().count() as i32;
        let lx = x + w - lw - 1;
        p.fill(
            lx - 1,
            r,
            lw + 2,
            1,
            blend(p.pal.background, p.pal.success, 0.35),
        );
        p.put(lx, r, label, p.pal.foreground);
        p.hit(lx - 1, r, lw + 2, 1, Action::Listen(si));
    }
    r += 2;
    let list = stories(d, cfg, v.tab, now);
    if list.is_empty() {
        let msg = if d.news.iter().all(Vec::is_empty) {
            "Waiting for Sveriges Radio…"
        } else {
            "No local news right now."
        };
        p.put(x + 1, r, msg, p.pal.muted);
        return (r + 2 - row).max(avail);
    }
    let end = if avail > 0 { row + avail } else { i32::MAX };
    let today = local(now).date();
    for (i, s) in list.iter().enumerate() {
        if r >= end {
            break;
        }
        let lt = local(s.at);
        let when = if lt.date() == today {
            sv::time(lt)
        } else {
            format!(
                "{} {}",
                sv::WEEKDAYS_SHORT[lt.format("%u").to_string().parse::<usize>().unwrap_or(1) - 1],
                sv::time(lt)
            )
        };
        let sel = i == v.selected;
        let lines = if sel {
            wrap(&s.title, (w - 8).max(10) as usize)
        } else {
            vec![s.title.clone()]
        };
        let mut extra = Vec::new();
        if sel {
            extra = wrap(&s.text, (w - 8).max(10) as usize);
            extra.truncate(4);
        }
        let h = lines.len() as i32 + extra.len() as i32 + i32::from(sel);
        if sel {
            p.fill(x, r, w, h, p.pal.selected);
        }
        p.put(
            x,
            r,
            &format!("{when:>9}"),
            if s.local { p.pal.warning } else { p.pal.muted },
        );
        if s.local {
            p.put(x + 10, r, "●", p.pal.warning);
        }
        for (k, l) in lines.iter().enumerate() {
            p.put_max(x + 12, r + k as i32, l, w - 13, p.pal.foreground);
        }
        for (k, l) in extra.iter().enumerate() {
            p.put_max(
                x + 12,
                r + lines.len() as i32 + k as i32,
                l,
                w - 13,
                p.pal.muted,
            );
        }
        if sel {
            let src = cfg
                .news
                .get(s.source)
                .map(|n| n.name.as_str())
                .unwrap_or("");
            p.put_max(
                x + 12,
                r + h - 1,
                &format!("{src} · Enter opens the article"),
                w - 13,
                p.pal.mark(),
            );
        }
        p.hit(x, r, w, h, Action::Story(i));
        r += h;
    }
    (r - row).max(avail)
}

/// Which source "Listen" plays for a tab: the tab's own, or the first.
pub fn listen_source(cfg: &Config, tab: usize) -> Option<usize> {
    let i = tab.saturating_sub(2);
    cfg.news
        .get(i)
        .filter(|n| n.channel != 0 || n.latest != 0)
        .map(|_| i)
        .or_else(|| {
            cfg.news
                .iter()
                .position(|n| n.channel != 0 || n.latest != 0)
        })
}

// ---------------------------------------------------------------- alerts

#[allow(clippy::too_many_arguments)]
fn panel_alerts(
    p: &mut P,
    x: i32,
    row: i32,
    w: i32,
    _avail: i32,
    d: &Data,
    cfg: &Config,
    _v: &View,
    now: i64,
) -> i32 {
    p.title(x, row, w, 103, "ALERTS", &cfg.county_name);
    let mut r = row + 1;
    for a in &d.vma {
        let bw = p.badge(x, r, if a.test { "VMA TEST" } else { "VMA" }, p.pal.error);
        p.put_max(x + bw + 1, r, &a.area, w - bw - 1, p.pal.foreground);
        for l in wrap(&a.text, (w - 2) as usize).iter().take(3) {
            r += 1;
            p.put_max(x + 1, r, l, w - 1, p.pal.foreground);
        }
        if !a.web.is_empty() {
            p.hit(x, r - 3, w, 4, Action::Open(a.web.clone()));
        }
        r += 1;
    }
    let active: Vec<&Warning> = d
        .warnings
        .iter()
        .filter(|wn| wn.end.is_none_or(|e| e > now))
        .collect();
    for wn in active.iter().take(6) {
        let (bg, label) = level_colour(&p.pal, wn.level);
        let bw = p.badge(x, r, label, bg);
        let when = match (wn.start, wn.end) {
            (Some(a), Some(b)) => format!(" {}–{}", short_when(a, now), short_when(b, now)),
            (Some(a), None) => format!(" from {}", short_when(a, now)),
            _ => String::new(),
        };
        let line = format!("{} · {}{}", wn.event, wn.area, when);
        p.put_max(
            x + bw + 1,
            r,
            &line,
            w - bw - 1,
            if wn.level == Level::Message {
                p.pal.muted
            } else {
                p.pal.foreground
            },
        );
        p.hit(
            x,
            r,
            w,
            1,
            Action::Open(
                "https://www.smhi.se/vader/varningar-och-brandrisk/varningar-och-meddelanden"
                    .into(),
            ),
        );
        r += 1;
    }
    if d.vma.is_empty() && active.iter().all(|w| w.level == Level::Message) {
        p.put(x, r, "✓", p.pal.success);
        p.put_max(
            x + 2,
            r,
            &format!("No VMA or weather warning for {}", cfg.county_name),
            w - 2,
            p.pal.success,
        );
        r += 1;
    }
    let recent: Vec<&Notice> = d
        .kris
        .iter()
        .filter(|n| now - n.at < 2 * 86400)
        .take(3)
        .collect();
    for n in recent {
        let bg = if n.local {
            p.pal.warning
        } else {
            blend(p.pal.background, p.pal.muted, 0.45)
        };
        let bw = p.badge(x, r, "KRIS", bg);
        p.put_max(x + bw + 1, r, &n.title, w - bw - 1, p.pal.foreground);
        if !n.link.is_empty() {
            p.hit(x, r, w, 1, Action::Open(n.link.clone()));
        }
        r += 1;
    }
    r - row
}

fn short_when(t: i64, now: i64) -> String {
    let lt = local(t);
    if lt.date() == local(now).date() {
        sv::time(lt)
    } else {
        format!("{} {}", sv::date_short(lt.date()), sv::time(lt))
    }
}

// ---------------------------------------------------------------- power

#[allow(clippy::too_many_arguments)]
fn panel_power(
    p: &mut P,
    x: i32,
    row: i32,
    w: i32,
    _avail: i32,
    d: &Data,
    cfg: &Config,
    v: &View,
    now: i64,
) -> i32 {
    let which = usize::from(v.tomorrow && !d.prices[1].is_empty());
    p.title(
        x,
        row,
        w,
        104,
        "ELECTRICITY",
        &format!("{} · spot, öre/kWh excl. VAT", cfg.price_area),
    );
    let mut r = row + 1;
    let prices = &d.prices[which];
    if prices.is_empty() {
        p.put(x + 1, r, "Waiting for elpriset…", p.pal.muted);
        return 2;
    }
    let hours = sources::hourly(prices);
    let lo = hours.iter().map(|h| h.1).fold(f64::INFINITY, f64::min);
    let hi = hours.iter().map(|h| h.1).fold(f64::NEG_INFINITY, f64::max);
    let mean = hours.iter().map(|h| h.1).sum::<f64>() / hours.len() as f64;
    let current = prices.iter().find(|q| q.from <= now && now < q.to);
    if which == 0 {
        if let Some(q) = current {
            let c = price_colour(&p.pal, q.ore, lo, hi);
            let tw = p.big_text(x * p.cw, r, &format!("{:.0}", q.ore), p.big, c);
            p.put(x + tw / p.cw + 1, r, "öre now", p.pal.muted);
        }
    } else {
        p.put(x, r, "TOMORROW", p.pal.mark());
    }
    let summary = format!("low {:.0} · mean {:.0} · high {:.0}", lo, mean, hi);
    let sw = summary.chars().count() as i32;
    if w - sw > 16 {
        p.put(x + w - sw - 1, r, &summary, p.pal.muted);
        r += 2;
    } else {
        r += 2;
        p.put_max(x, r, &summary, w, p.pal.muted);
        r += 1;
    }
    // Bars: one per hour, six rows high.
    let chart_rows = 6;
    let n = hours.len() as i32;
    let bw = ((w - 1) / n.max(1)).max(1);
    if p.visible(r, chart_rows) {
        let area = p.cells(x, r, bw * n, chart_rows);
        let scale_lo = lo.min(0.0);
        for (i, (start, ore)) in hours.iter().enumerate() {
            let hgt = (((ore - scale_lo) / (hi - scale_lo).max(1.0)) * area.h as f64)
                .round()
                .max(2.0) as i32;
            let bx = area.x + i as i32 * bw * p.cw;
            let c = price_colour(&p.pal, *ore, lo, hi);
            let now_hour = which == 0 && *start <= now && now < start + 3600;
            let c = if now_hour { p.pal.foreground } else { c };
            let bar =
                Rect::new(bx + 1, area.y + area.h - hgt, bw * p.cw - 2, hgt).intersect(&p.body);
            p.cv.fill(bar, c);
        }
    }
    r += chart_rows;
    for (i, (start, _)) in hours.iter().enumerate() {
        let h = local(*start).hour();
        if h % 6 == 0 {
            p.put(x + i as i32 * bw, r, &format!("{h:02}"), p.pal.muted);
        }
    }
    r += 1;
    if let Some((start, m)) = sources::cheapest(&hours, 3, if which == 0 { now } else { 0 }) {
        let a = local(start).hour();
        p.put_max(
            x,
            r,
            &format!("Cheapest 3 h: {:02}–{:02}, {:.0} öre", a, (a + 3) % 24, m),
            w,
            p.pal.success,
        );
        r += 1;
    }
    if which == 0 && !d.prices[1].is_empty() {
        let th = sources::hourly(&d.prices[1]);
        let tm = th.iter().map(|h| h.1).sum::<f64>() / th.len().max(1) as f64;
        p.put_max(
            x,
            r,
            &format!("Tomorrow: mean {tm:.0} öre · T shows it"),
            w,
            p.pal.muted,
        );
        p.hit(x, r, w, 1, Action::Tomorrow);
        r += 1;
    } else if which == 1 {
        p.put_max(x, r, "T shows today", w, p.pal.muted);
        p.hit(x, r, w, 1, Action::Tomorrow);
        r += 1;
    }
    r - row
}

// ---------------------------------------------------------------- today

#[allow(clippy::too_many_arguments)]
fn panel_today(
    p: &mut P,
    x: i32,
    row: i32,
    w: i32,
    _avail: i32,
    d: &Data,
    cfg: &Config,
    _v: &View,
    now: i64,
) -> i32 {
    let today: NaiveDate = local(now).date();
    p.title(x, row, w, 105, "TODAY", &sv::date_long(today));
    let mut r = row + 1;
    let label = |p: &mut P, r: i32, l: &str| {
        p.put(x, r, l, p.pal.mark());
    };
    if let Some(day) = &d.day {
        if !day.names.is_empty() {
            label(p, r, "Name day");
            p.put_max(
                x + 11,
                r,
                &sv::and_list(&day.names),
                w - 11,
                p.pal.foreground,
            );
            r += 1;
        }
        if !day.holiday.is_empty() {
            label(p, r, "Holiday");
            p.put_max(x + 11, r, &day.holiday, w - 11, p.pal.error);
            r += 1;
        }
        if !day.flag.is_empty() {
            label(p, r, "Flag day");
            p.put_max(x + 11, r, &day.flag, w - 11, p.pal.foreground);
            r += 1;
        }
    }
    let s = crate::sun::sun(today, cfg.latitude, cfg.longitude);
    let y = crate::sun::sun(
        today.pred_opt().unwrap_or(today),
        cfg.latitude,
        cfg.longitude,
    );
    label(p, r, "Sun");
    let diff = s.day_minutes - y.day_minutes;
    let line = match (s.rise, s.set) {
        (Some(a), Some(b)) => format!(
            "↑ {}  ↓ {}  {} ({}{})",
            sv::time(local(a)),
            sv::time(local(b)),
            sv::duration(s.day_minutes),
            if diff < 0 { "\u{2212}" } else { "+" },
            sv::duration(diff)
        ),
        _ if s.day_minutes > 0 => "midnight sun".to_string(),
        _ => "polar night".to_string(),
    };
    p.put_max(x + 11, r, &line, w - 11, p.pal.foreground);
    r += 1;
    // Calendar.
    if cfg.calendar {
        r += 1;
        label(p, r, "CALENDAR");
        r += 1;
        let tomorrow = today.succ_opt().unwrap_or(today);
        let todays: Vec<&Entry> = d
            .calendar
            .iter()
            .filter(|e| {
                day_of(&e.span) <= today
                    && today < end_of(&e.span).max(day_of(&e.span).succ_opt().unwrap_or(today))
            })
            .collect();
        let next: Vec<&Entry> = d
            .calendar
            .iter()
            .filter(|e| day_of(&e.span) == tomorrow)
            .take(3)
            .collect();
        if todays.is_empty() {
            let note = d
                .calendar_note
                .clone()
                .unwrap_or_else(|| "Nothing in the calendar today".to_string());
            p.put_max(x + 1, r, &note, w - 1, p.pal.muted);
            r += 1;
        }
        for e in todays.iter().take(8) {
            entry_line(p, x, r, w, e, now);
            r += 1;
        }
        if !next.is_empty() {
            p.put(x, r, "Tomorrow", p.pal.muted);
            r += 1;
            for e in next {
                entry_line(p, x, r, w, e, now);
                r += 1;
            }
        }
    }
    r - row
}

fn day_of(s: &Span) -> NaiveDate {
    match *s {
        Span::AllDay(a, _) => a,
        Span::Timed(a, _) => local(a).date(),
    }
}
fn end_of(s: &Span) -> NaiveDate {
    match *s {
        Span::AllDay(_, b) => b,
        Span::Timed(_, b) => local(b - 1).date().succ_opt().unwrap_or_default(),
    }
}

fn entry_line(p: &mut P, x: i32, r: i32, w: i32, e: &Entry, now: i64) {
    let col = e.colour.unwrap_or(p.pal.mark());
    p.fill(x, r, 1, 1, col);
    let (when, past) = match e.span {
        Span::AllDay(..) => ("all day".to_string(), false),
        Span::Timed(a, b) => (
            format!("{}–{}", sv::time(local(a)), sv::time(local(b))),
            b <= now,
        ),
    };
    p.put(
        x + 2,
        r,
        &format!("{when:<11}"),
        if past { p.pal.muted } else { p.pal.foreground },
    );
    let mut what = e.summary.clone();
    if !e.location.is_empty() {
        what = format!("{what} · {}", e.location);
    }
    p.put_max(
        x + 14,
        r,
        &what,
        w - 14,
        if past { p.pal.muted } else { p.pal.foreground },
    );
}

// ---------------------------------------------------------------- traffic

#[allow(clippy::too_many_arguments)]
fn panel_traffic(
    p: &mut P,
    x: i32,
    row: i32,
    w: i32,
    _avail: i32,
    d: &Data,
    cfg: &Config,
    _v: &View,
    _now: i64,
) -> i32 {
    p.title(
        x,
        row,
        w,
        106,
        "TRAFFIC",
        &format!("Trafikverket · {} km", cfg.traffic_radius_km),
    );
    let mut r = row + 1;
    if cfg.traffic_key.trim().is_empty() {
        for l in wrap("Add a free Trafikverket key: [traffic] key in ~/.config/nirutoday/config.toml (data.trafikverket.se).", (w - 1) as usize) {
            p.put_max(x + 1, r, &l, w - 1, p.pal.muted);
            r += 1;
        }
        return r - row;
    }
    if let Some(n) = &d.traffic_note {
        p.put_max(x + 1, r, n, w - 1, p.pal.warning);
        r += 1;
    }
    if d.traffic.is_empty() && d.traffic_note.is_none() {
        p.put(x, r, "✓", p.pal.success);
        p.put_max(x + 2, r, "No disruptions nearby", w - 2, p.pal.success);
        return r + 1 - row;
    }
    for i in d.traffic.iter().take(8) {
        let bg = match i.severity_code {
            5 => p.pal.error,
            4 => blend(p.pal.warning, p.pal.error, 0.5),
            2 => p.pal.warning,
            _ => blend(p.pal.background, p.pal.muted, 0.45),
        };
        let road = if i.road.is_empty() {
            "•".to_string()
        } else {
            i.road.clone()
        };
        let bw = p.badge(x, r, &road, bg);
        let kind = if i.kind.is_empty() {
            &i.header
        } else {
            &i.kind
        };
        p.put_max(
            x + bw + 1,
            r,
            &format!("{kind} · {}", i.place),
            w - bw - 1,
            p.pal.foreground,
        );
        r += 1;
        if !i.text.is_empty() {
            p.put_max(x + bw + 1, r, &i.text, w - bw - 1, p.pal.muted);
            r += 1;
        }
    }
    r - row
}

// ---------------------------------------------------------------- help

fn help(p: &mut P, full: Rect, cols: i32) {
    let lines = [
        ("R", "refresh everything now"),
        ("↑ ↓  J K", "choose a news story"),
        ("Enter", "open the story in the browser"),
        ("Tab  Shift+Tab", "all, local, one program"),
        ("L", "listen: live or the newest broadcast, in NIRUAMP"),
        ("T", "electricity prices tomorrow / today"),
        ("PgUp PgDn Home", "scroll the page"),
        ("+  −  0", "larger, smaller, normal"),
        ("F  F11", "full screen"),
        ("Q  Esc", "quit"),
        ("", ""),
        ("Click", "a story, a tab, LIVE, an alert"),
    ];
    let w = 56.min(cols - 2);
    let h = lines.len() as i32 + 4;
    let x = (cols - w) / 2;
    let top = ((full.h / p.ch) - h) / 2;
    let r = Rect::new(x * p.cw, top * p.ch, w * p.cw, h * p.ch);
    p.cv.fill(r, p.pal.surface);
    p.cv.frame(r, p.pal.mark());
    let save = (p.dy, p.body);
    p.dy = 0;
    p.body = full;
    p.body.y = 0;
    p.title(x, top, w, 199, "KEYS", "? closes");
    for (i, (k, d)) in lines.iter().enumerate() {
        p.put(x + 2, top + 2 + i as i32, k, p.pal.warning);
        p.put_max(x + 19, top + 2 + i as i32, d, w - 20, p.pal.foreground);
    }
    (p.dy, p.body) = save;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_words_and_cuts_long_ones() {
        assert_eq!(wrap("ett två tre fyra", 7), vec!["ett två", "tre", "fyra"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert!(wrap("", 5).is_empty());
    }

    #[test]
    fn local_stories_first_then_time_order_and_no_duplicates() {
        let cfg = Config::default();
        let s = |source, at, title: &str, local| Story {
            source,
            at,
            title: title.into(),
            text: String::new(),
            link: String::new(),
            local,
        };
        let d = Data {
            news: vec![
                vec![s(0, 100, "Lokal", true), s(0, 300, "Ny", false)],
                vec![s(1, 200, "Riks", false), s(1, 300, "ny", false)],
            ],
            ..Data::default()
        };
        let all = stories(&d, &cfg, 0, 400);
        assert_eq!(
            all.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            vec!["Lokal", "Ny", "Riks"]
        );
        assert_eq!(stories(&d, &cfg, 1, 400).len(), 1);
        assert_eq!(stories(&d, &cfg, 3, 400).len(), 2);
        assert_eq!(listen_source(&cfg, 0), Some(0));
        assert_eq!(listen_source(&cfg, 3), Some(1));
    }
}
