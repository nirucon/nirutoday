//! NIRU suite colours.
//!
//! The same contract as the other NIRU components: the theme named in
//! `~/.config/niruwm/config.toml`, a user theme file in
//! `~/.config/niruwm/themes/NAME.toml` before the built-in palette, and the
//! optional `[colors]` of `~/.config/niruwm/appearance.toml`. NIRUWM publishes
//! the theme in effect as `_NIRUWM_THEME` on the root window; a change of that
//! property reloads the palette live.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: u32,
    pub surface: u32,
    pub foreground: u32,
    pub muted: u32,
    pub border: u32,
    pub accent: u32,
    pub selected: u32,
    pub warning: u32,
    pub error: u32,
    pub success: u32,
}

impl Default for Palette {
    fn default() -> Self {
        builtin("niru-noir").unwrap()
    }
}

impl Palette {
    /// A light theme (Carl Larsson) needs the light icon set.
    pub fn is_light(&self) -> bool {
        luminance(self.background) > 0.5
    }

    /// The accent, lifted towards the text when it is too close to the
    /// ground to mark anything (Noir's grey focus colour).
    pub fn mark(&self) -> u32 {
        if (luminance(self.accent) - luminance(self.background)).abs() < 0.35 {
            blend(self.accent, self.foreground, 0.55)
        } else {
            self.accent
        }
    }
}

pub fn blend(a: u32, b: u32, amount: f32) -> u32 {
    let mix = |x: u32, y: u32| {
        let x = (x & 0xff) as f32;
        let y = (y & 0xff) as f32;
        (x + (y - x) * amount).round().clamp(0.0, 255.0) as u32
    };
    (mix(a >> 16, b >> 16) << 16) | (mix(a >> 8, b >> 8) << 8) | mix(a, b)
}

fn luminance(c: u32) -> f32 {
    let r = ((c >> 16) & 0xff) as f32 / 255.0;
    let g = ((c >> 8) & 0xff) as f32 / 255.0;
    let b = (c & 0xff) as f32 / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn builtin(name: &str) -> Option<Palette> {
    let (background, foreground, muted, border, accent) = match name {
        "niru-noir" => (0x0f0f10, 0xe5e5e5, 0xa8a8a8, 0x2a2a2d, 0x5a5a60),
        "niru-nord" => (0x2e3440, 0xeceff4, 0x9aa5b1, 0x3b4252, 0x88c0d0),
        "niru-satie" => (0x141414, 0xdedad4, 0x8f8b86, 0x2a2724, 0xb08d7d),
        "niru-carl-larsson" => (0xf3ece0, 0x3b3228, 0x7d6f5f, 0xd8cdbb, 0xa63d2f),
        "niru-forest-vigil" => (0x0b100c, 0xd8ded2, 0x96a292, 0x2b362c, 0x8aa87e),
        "niru-blood-of-urdr" => (0x120808, 0xe8d6cf, 0xb09a92, 0x3a1d1b, 0xc96050),
        "niru-root-and-stone" => (0x17100b, 0xeadfd2, 0xb5a08c, 0x3a2a1e, 0xc19768),
        "niru-blue-horizon" => (0x0b1821, 0xdce6eb, 0x93a6b2, 0x243846, 0x86aec8),
        "niru-the-norns" => (0x111110, 0xece9e4, 0xa6a19b, 0x3b3936, 0xc9c2b9),
        "niru-winter-nights" => (0x1f232b, 0xead9c3, 0xb9aa9a, 0x4e5157, 0xd8c4aa),
        _ => return None,
    };
    Some(Palette {
        background,
        surface: blend(background, foreground, 0.05),
        foreground,
        muted,
        border,
        accent,
        selected: blend(background, accent, 0.28),
        warning: 0xd7b36a,
        error: 0xd77a7a,
        success: 0x8fbf7f,
    })
}

pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches('#');
    (s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(s, 16).ok())
        .flatten()
}

fn read_toml(path: &Path) -> Option<toml::Value> {
    std::fs::read_to_string(path)
        .ok()?
        .parse::<toml::Value>()
        .ok()
}

/// The theme name from the WM (`_NIRUWM_THEME`) or the configuration file.
pub fn configured_theme(base: &Path) -> String {
    read_toml(&base.join("niruwm/config.toml"))
        .and_then(|v| v.get("theme")?.as_str().map(str::to_string))
        .unwrap_or_else(|| "niru-noir".to_string())
}

/// The font size of the suite (`font_size` in the NIRUWM configuration).
pub fn configured_font_size(base: &Path) -> Option<f32> {
    let v = read_toml(&base.join("niruwm/config.toml"))?;
    let size = v.get("font_size")?;
    size.as_float()
        .or_else(|| size.as_integer().map(|i| i as f32 as f64))
        .map(|f| f as f32)
        .filter(|f| (8.0..=40.0).contains(f))
}

pub fn configured_font_path(base: &Path) -> Option<String> {
    let v = read_toml(&base.join("niruwm/config.toml"))?;
    let p = v.get("font_path")?.as_str()?;
    let p = p
        .strip_prefix("~/")
        .map_or_else(|| PathBuf::from(p), |rest| home().join(rest));
    p.is_file().then(|| p.to_string_lossy().into_owned())
}

pub fn load(base: &Path, name: &str) -> Palette {
    let mut p = builtin(name).unwrap_or_default();
    let mut apply = |table: &toml::Value| {
        let get = |key: &str, old: u32| {
            table
                .get(key)
                .and_then(|v| v.as_str())
                .and_then(parse_hex)
                .unwrap_or(old)
        };
        p.background = get("background", p.background);
        p.surface = get("surface", p.surface);
        p.foreground = get("foreground", p.foreground);
        p.muted = get("muted", p.muted);
        p.border = get("border", p.border);
        p.accent = get("accent", get("focus", p.accent));
        p.selected = get("selected", p.selected);
        p.warning = get("warning", p.warning);
        p.error = get("error", p.error);
        p.success = get("success", p.success);
    };
    let theme_file = base.join("niruwm/themes").join(format!("{name}.toml"));
    if let Some(theme) = read_toml(&theme_file) {
        apply(theme.get("colors").unwrap_or(&theme));
    } else if let Some(appearance) = read_toml(&base.join("niruwm/appearance.toml")) {
        if appearance.get("schema").and_then(|v| v.as_integer()) == Some(1) {
            if let Some(colors) = appearance.get("colors") {
                apply(colors);
            }
        }
    }
    if p.is_light() {
        // The built-in signal colours are made for a dark ground.
        if p.warning == 0xd7b36a {
            p.warning = 0x9a6a00;
        }
        if p.error == 0xd77a7a {
            p.error = 0xa02020;
        }
        if p.success == 0x8fbf7f {
            p.success = 0x3a7a2a;
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_theme_loads_and_carl_larsson_is_light() {
        for name in [
            "niru-noir",
            "niru-nord",
            "niru-forest-vigil",
            "niru-winter-nights",
        ] {
            assert!(builtin(name).is_some(), "{name}");
        }
        assert!(builtin("niru-carl-larsson").unwrap().is_light());
        assert!(!builtin("niru-noir").unwrap().is_light());
    }

    #[test]
    fn a_user_theme_file_wins_over_the_builtin_palette() {
        let dir = std::env::temp_dir().join(format!("nirutoday-theme-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("niruwm/themes")).unwrap();
        std::fs::write(
            dir.join("niruwm/themes/mine.toml"),
            "[colors]\nbackground = \"101010\"\naccent = \"#ff8800\"\n",
        )
        .unwrap();
        let p = load(&dir, "mine");
        assert_eq!(p.background, 0x101010);
        assert_eq!(p.accent, 0xff8800);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
