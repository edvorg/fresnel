//! The colors of the escape sequences of a log (the SGR codes, research 05 section 5.3).
//!
//! The 16 standard colors have one set for each theme: the same yellow cannot be read on a
//! dark and on a light background. The cube of colors and the grays of the 256 colors and
//! each RGB color are what the program asked for.

use egui::Color32;
use fresnel_core::logs::Color;

/// The standard colors and their bright forms on a dark background.
const DARK: [[u8; 3]; 16] = [
    [0x4d, 0x4d, 0x4d],
    [0xf1, 0x4c, 0x4c],
    [0x23, 0xd1, 0x8b],
    [0xe5, 0xe5, 0x10],
    [0x3b, 0x8e, 0xea],
    [0xd6, 0x70, 0xd6],
    [0x29, 0xb8, 0xdb],
    [0xcc, 0xcc, 0xcc],
    [0x76, 0x76, 0x76],
    [0xf5, 0x6d, 0x6d],
    [0x3e, 0xe6, 0xa0],
    [0xf5, 0xf5, 0x43],
    [0x6c, 0xad, 0xf5],
    [0xe2, 0x91, 0xe2],
    [0x55, 0xd0, 0xee],
    [0xf2, 0xf2, 0xf2],
];

/// The standard colors and their bright forms on a light background.
const LIGHT: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xcd, 0x31, 0x31],
    [0x10, 0x7c, 0x10],
    [0x94, 0x7d, 0x00],
    [0x04, 0x51, 0xa5],
    [0xbc, 0x05, 0xbc],
    [0x05, 0x86, 0xa8],
    [0x55, 0x55, 0x55],
    [0x66, 0x66, 0x66],
    [0xe0, 0x3e, 0x3e],
    [0x14, 0x9e, 0x14],
    [0xa8, 0x90, 0x00],
    [0x1f, 0x6f, 0xc4],
    [0xd0, 0x2c, 0xd0],
    [0x0a, 0x9c, 0xc0],
    [0x8a, 0x8a, 0x8a],
];

/// The six levels of each part of a color of the cube.
const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The color of the program in the theme of the pane.
pub(crate) fn of(color: Color, dark: bool) -> Color32 {
    match color {
        Color::Rgb(red, green, blue) => Color32::from_rgb(red, green, blue),
        Color::Palette(index @ 0..=15) => {
            let set = if dark { &DARK } else { &LIGHT };
            let [red, green, blue] = set.get(usize::from(index)).copied().unwrap_or([0x80; 3]);
            Color32::from_rgb(red, green, blue)
        }
        Color::Palette(index @ 16..=231) => {
            let cube = index - 16;
            let level = |part: u8| LEVELS.get(usize::from(part)).copied().unwrap_or(0);
            Color32::from_rgb(level(cube / 36), level(cube / 6 % 6), level(cube % 6))
        }
        Color::Palette(gray) => {
            let value = 8 + 10 * (gray - 232);
            Color32::from_rgb(value, value, value)
        }
    }
}

/// The color of bold text: a standard color becomes its bright form, as in most terminals.
/// The monospace font of the app has no bold face.
pub(crate) fn bold(color: Option<Color>) -> Option<Color> {
    match color {
        Some(Color::Palette(index @ 0..=7)) => Some(Color::Palette(index + 8)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_256_colors_have_their_cube_and_grays_and_the_standard_ones_follow_the_theme() {
        assert_eq!(of(Color::Palette(16), true), Color32::from_rgb(0, 0, 0));
        assert_eq!(of(Color::Palette(196), true), Color32::from_rgb(255, 0, 0));
        assert_eq!(of(Color::Palette(46), false), Color32::from_rgb(0, 255, 0));
        assert_eq!(of(Color::Palette(109), true), Color32::from_rgb(135, 175, 175));
        assert_eq!(
            (of(Color::Palette(232), true), of(Color::Palette(255), true)),
            (Color32::from_gray(8), Color32::from_gray(238))
        );
        assert_eq!(of(Color::Rgb(1, 2, 3), false), Color32::from_rgb(1, 2, 3));
        assert_ne!(of(Color::Palette(3), true), of(Color::Palette(3), false), "a yellow for each background");
        assert_eq!(
            (bold(Some(Color::Palette(1))), bold(Some(Color::Palette(9)))),
            (Some(Color::Palette(9)), Some(Color::Palette(9)))
        );
        assert_eq!(bold(None), None);
    }
}
