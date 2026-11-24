# NIRUTODAY

Today at a glance for [NIRUX](https://github.com/nirucon/nirux), in a retro
teletext look: the weather from SMHI, news from Sveriges Radio, VMA and
warnings, the electricity price, the name day, your calendar and, if you want
it, traffic from Trafikverket. Rust and X11, in the NIRU theme, a few
megabytes of memory.

## What it shows

- **Weather (101):** now, the next hours and the week from SMHI, with
  teletext mosaic pictures, "feels like", wind, rain and a temperature range
  bar per day.
- **News (102):** articles from Sveriges Radio programs, P4 Väst and Ekot by
  default. News that names a nearby place is marked and listed first. Tabs:
  all, local, one program. **Listen** plays the channel live (P4 Väst) or the
  newest broadcast (Ekot) in NIRUAMP, which is started when it is not running.
- **Alerts (103):** VMA (Viktigt meddelande till allmänheten) for your
  county as a red banner, SMHI's weather warnings and Krisinformation.se. A
  new VMA, or an orange or red warning, also comes as a notice, once.
- **Electricity (104):** the spot price per hour for your elområde as a bar
  chart, the price now, and the cheapest three hours. Tomorrow's prices
  after about 13:00 (`T`).
- **Today (105):** name day, flag day and holiday, sunrise and sunset with
  the change since yesterday, and today's events from the calendars NIRUBAR
  reads.
- **Traffic (106, off by default):** Trafikverket's disruptions within a
  radius of your place. It needs a free key from
  [Trafikverket's open data](https://data.trafikverket.se/).

Dates and times are Swedish: "tisdag 24 november", week 48, 24 hour clock and
a decimal comma. The page has three columns on a wide screen, two on a laptop
and one in a narrow window or a scratchpad, and grows its text on large
screens.

## Keys

| Key | What it does |
|---|---|
| `R`, `F5` | refresh everything now |
| `Up`, `Down`, `J`, `K` | choose a news story |
| `Enter` | open the story in the browser |
| `Tab`, `Shift+Tab` | all, local, one program |
| `L` | listen (live or the newest broadcast) in NIRUAMP |
| `T` | electricity prices tomorrow and back |
| `PgUp`, `PgDn`, `Home`, `End` | scroll the page |
| `+`, `-`, `0` | larger, smaller, normal |
| `F`, `F11` | full screen |
| `?`, `F1` | all keys |
| `Q`, `Esc` | quit |

Clicks work too: a story, a tab, **LIVE**, an alert. `nirutoday --print`
writes today as text, for a terminal.

## Settings

`~/.config/nirutoday/config.toml` is written at the first start with every
setting and a comment. The defaults are Ljungskile (Uddevalla municipality,
Västra Götaland), P4 Väst and Ekot, elområde SE3, and traffic off:

```toml
[place]
name = "Ljungskile"
latitude = 58.2235
longitude = 11.9211
county = 14                       # län number, for VMA, warnings and traffic
nearby = ["Ljungskile", "Uddevalla", "Stenungsund", "Orust"]

[[news.source]]
name = "P4 Väst"
program = 125                     # Sveriges Radio program with the articles
channel = 219                     # channel played live by Listen

[electricity]
area = "SE3"

[traffic]
enabled = false
key = ""                          # keep this file private (chmod 600)
radius_km = 30
```

Colours and the font follow the NIRU theme (`~/.config/niruwm/config.toml`
and the `_NIRUWM_THEME` root property) and change live with it.

## How it works

Every source is fetched with `curl` in the background and kept in
`~/.cache/nirutoday/`, so the page opens at once with the last data and
works offline. Each source has its own pace (VMA every 2 minutes, news every
5, the forecast every 30). A hidden window (another workspace, a closed
scratchpad) fetches only VMA and warnings, so a notice still comes.

Sources: [SMHI open data](https://opendata.smhi.se/) (forecast and
warnings), [Sveriges Radio's API](https://api.sr.se/) (news and VMA),
[Krisinformation.se](https://www.krisinformation.se/),
[elprisetjustnu.se](https://www.elprisetjustnu.se/) (spot prices),
[dryg.net](https://sholiday.faboul.se/) (name days) and, when you turn it on,
[Trafikverket](https://data.trafikverket.se/). Your place goes to SMHI (the
forecast point) and, with traffic on, to Trafikverket; your calendar
addresses are only used to fetch your own calendars.

## Install

NIRUTODAY is a NIRUX extra: `nirux extras install nirutoday`. On its own,
from a release: put `nirutoday` in your `PATH`, and `nirutoday.desktop` and
`nirutoday.svg` where your desktop finds entries and icons. It needs `curl`.

```sh
cargo build --release
sh tests/xvfb.sh target/release/nirutoday   # end to end in a private Xvfb
```

## Licence

MIT.
