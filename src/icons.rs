//! Weather pictures in teletext mosaic style: 16 x 12 blocks per picture,
//! one per SMHI symbol (Wsymb2, 1 to 27), drawn at any size in whole blocks.
use crate::canvas::{Canvas, Rect};
use crate::theme::{blend, Palette};

const SUN: [&str; 12] = [
    ".......y........",
    "..y....y....y...",
    "...y.......y....",
    ".....yyyyy......",
    "....yyyyyyy.....",
    "yy.yyyyyyyyy.yy.",
    "....yyyyyyy.....",
    ".....yyyyy......",
    "...y.......y....",
    "..y....y....y...",
    ".......y........",
    "................",
];
const MOON: [&str; 12] = [
    "................",
    "......mmmm......",
    "....mmmm.....m..",
    "...mmmm.........",
    "...mmm..........",
    "...mmm......m...",
    "...mmmm.........",
    "....mmmmm...m...",
    ".....mmmmmmm....",
    ".......mmm......",
    "................",
    "................",
];
const CLOUD: [&str; 12] = [
    "................",
    "................",
    "................",
    ".......wwww.....",
    ".....wwwwwwww...",
    "..www.wwwwwwww..",
    ".wwwwwwwwwwwwww.",
    ".wwwwwwwwwwwwww.",
    "..wwwwwwwwwwww..",
    "................",
    "................",
    "................",
];
const PARTLY: [&str; 12] = [
    "..y..y..........",
    "...yyyy.........",
    ".yyyyyyy........",
    "..yyyyywwww.....",
    ".yyyywwwwwwww...",
    "..yyw.wwwwwwww..",
    "...wwwwwwwwwwww.",
    ".wwwwwwwwwwwwww.",
    "..wwwwwwwwwwww..",
    "................",
    "................",
    "................",
];
const PARTLY_NIGHT: [&str; 12] = [
    "..mmm...........",
    ".mmm............",
    ".mm.............",
    ".mm....wwww.....",
    ".mmmwwwwwwwww...",
    "..mmw.wwwwwwww..",
    "...wwwwwwwwwwww.",
    ".wwwwwwwwwwwwww.",
    "..wwwwwwwwwwww..",
    "................",
    "................",
    "................",
];
const FOG: [&str; 12] = [
    "................",
    "................",
    "..ffffffffffff..",
    "................",
    "ffffffffffff....",
    "................",
    "....ffffffffffff",
    "................",
    "..ffffffffffff..",
    "................",
    "ffffffffff......",
    "................",
];
const BOLT: [(usize, &str); 5] = [
    (7, "........tt......"),
    (8, ".......tt......."),
    (9, "......tttt......"),
    (10, "........t......."),
    (11, ".......t........"),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fall {
    None,
    Rain,
    Sleet,
    Snow,
}

/// What a symbol is made of: (base picture, dark cloud, precipitation,
/// rows of it 0-3, thunder).
fn parts(symbol: u8, day: bool) -> (&'static [&'static str; 12], bool, Fall, u8, bool) {
    let clear = if day { &SUN } else { &MOON };
    let partly = if day { &PARTLY } else { &PARTLY_NIGHT };
    match symbol {
        1 | 2 => (clear, false, Fall::None, 0, false),
        3 | 4 => (partly, false, Fall::None, 0, false),
        5 => (&CLOUD, false, Fall::None, 0, false),
        6 => (&CLOUD, true, Fall::None, 0, false),
        7 => (&FOG, false, Fall::None, 0, false),
        8 => (partly, false, Fall::Rain, 1, false),
        9 => (partly, false, Fall::Rain, 2, false),
        10 => (partly, true, Fall::Rain, 3, false),
        11 => (&CLOUD, true, Fall::Rain, 1, true),
        12 => (partly, false, Fall::Sleet, 1, false),
        13 => (partly, false, Fall::Sleet, 2, false),
        14 => (partly, true, Fall::Sleet, 3, false),
        15 => (partly, false, Fall::Snow, 1, false),
        16 => (partly, false, Fall::Snow, 2, false),
        17 => (partly, true, Fall::Snow, 3, false),
        18 => (&CLOUD, false, Fall::Rain, 1, false),
        19 => (&CLOUD, true, Fall::Rain, 2, false),
        20 => (&CLOUD, true, Fall::Rain, 3, false),
        21 => (&CLOUD, true, Fall::None, 0, true),
        22 => (&CLOUD, false, Fall::Sleet, 1, false),
        23 => (&CLOUD, true, Fall::Sleet, 2, false),
        24 => (&CLOUD, true, Fall::Sleet, 3, false),
        25 => (&CLOUD, false, Fall::Snow, 1, false),
        26 => (&CLOUD, true, Fall::Snow, 2, false),
        27 => (&CLOUD, true, Fall::Snow, 3, false),
        _ => (&CLOUD, false, Fall::None, 0, false),
    }
}

/// The 16 x 12 picture as colour letters.
pub fn picture(symbol: u8, day: bool) -> [[u8; 16]; 12] {
    let (base, dark, fall, rows, thunder) = parts(symbol, day);
    let mut g = [[b'.'; 16]; 12];
    for (y, row) in base.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            g[y][x] = if dark && c == b'w' { b'g' } else { c };
        }
    }
    // Precipitation under the cloud: row 10 first, then 9 and 11.
    let order = [10usize, 9, 11];
    for (n, &y) in order.iter().take(rows as usize).enumerate() {
        let shift = if y % 2 == 0 { 1 } else { 3 };
        for x in (shift..15).step_by(4) {
            let c = match fall {
                Fall::Rain => b'b',
                Fall::Snow => b's',
                Fall::Sleet => {
                    if (x / 4 + n) % 2 == 0 {
                        b'b'
                    } else {
                        b's'
                    }
                }
                Fall::None => continue,
            };
            g[y][x] = c;
        }
    }
    if thunder {
        for (y, row) in BOLT {
            for (x, c) in row.bytes().enumerate() {
                if c == b't' {
                    g[y][x] = b't';
                }
            }
        }
    }
    g
}

fn colour(c: u8, p: &Palette) -> Option<u32> {
    let light = p.is_light();
    Some(match c {
        b'y' => {
            if light {
                0xd89a00
            } else {
                0xf2c94c
            }
        }
        b'm' => {
            if light {
                0x8a7a40
            } else {
                0xe8dcb0
            }
        }
        b'w' => blend(p.foreground, p.background, 0.15),
        b'g' => blend(p.muted, p.background, 0.25),
        b'b' => {
            if light {
                0x1f6fd0
            } else {
                0x5aa9ff
            }
        }
        b's' => {
            if light {
                0x6a7a90
            } else {
                0xf4f8ff
            }
        }
        b't' => {
            if light {
                0xc08a00
            } else {
                0xffe04a
            }
        }
        b'f' => p.muted,
        _ => return None,
    })
}

/// Draw the picture in whole blocks, centred in `r`.
pub fn draw(cv: &mut Canvas, r: Rect, symbol: u8, day: bool, p: &Palette) {
    let b = (r.w / 16).min(r.h / 12).max(1);
    let bh = b;
    let (w, h) = (16 * b, 12 * bh);
    let (x0, y0) = (r.x + (r.w - w) / 2, r.y + (r.h - h) / 2);
    let g = picture(symbol, day);
    for (y, row) in g.iter().enumerate() {
        for (x, &c) in row.iter().enumerate() {
            if let Some(col) = colour(c, p) {
                cv.fill(Rect::new(x0 + x as i32 * b, y0 + y as i32 * bh, b, bh), col);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_symbol_has_a_picture_and_night_has_a_moon() {
        for s in 1..=27u8 {
            let g = picture(s, true);
            assert!(g.iter().flatten().any(|&c| c != b'.'), "symbol {s}");
        }
        assert!(picture(1, false).iter().flatten().any(|&c| c == b'm'));
        assert!(picture(1, true).iter().flatten().any(|&c| c == b'y'));
        assert!(
            picture(20, true)
                .iter()
                .flatten()
                .filter(|&&c| c == b'b')
                .count()
                > picture(18, true)
                    .iter()
                    .flatten()
                    .filter(|&&c| c == b'b')
                    .count()
        );
        assert!(picture(11, true).iter().flatten().any(|&c| c == b't'));
        assert!(picture(26, true).iter().flatten().any(|&c| c == b's'));
    }
}
