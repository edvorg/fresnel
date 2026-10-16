//! The colors of a terminal: the 16 named colors of a theme, the 6x6x6 cube and the gray
//! ramp of xterm, and the colors that the remote program set (OSC 4, 10, 11).

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{self, Color, NamedColor};

use crate::config::Rgb;

/// The colors of one theme of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    /// Black, red, green, yellow, blue, magenta, cyan and white, then their bright forms.
    pub ansi: [Rgb; 16],
}

const fn rgb(value: u32) -> Rgb {
    Rgb([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

impl Palette {
    /// The terminal colors of the dark theme: those of the terminal of VS Code, on the
    /// sunken background of the app.
    pub const DARK: Self = Self {
        foreground: rgb(0xd9dce3),
        background: rgb(0x111317),
        cursor: rgb(0xd9dce3),
        ansi: [
            rgb(0x000000),
            rgb(0xcd3131),
            rgb(0x0dbc79),
            rgb(0xe5e510),
            rgb(0x2472c8),
            rgb(0xbc3fbc),
            rgb(0x11a8cd),
            rgb(0xe5e5e5),
            rgb(0x666666),
            rgb(0xf14c4c),
            rgb(0x23d18b),
            rgb(0xf5f543),
            rgb(0x3b8eea),
            rgb(0xd670d6),
            rgb(0x29b8db),
            rgb(0xffffff),
        ],
    };

    /// The terminal colors of the light theme: those of the light theme of VS Code, which
    /// keep yellow and white readable on a light background.
    pub const LIGHT: Self = Self {
        foreground: rgb(0x1f2328),
        background: rgb(0xf8f9fb),
        cursor: rgb(0x1f2328),
        ansi: [
            rgb(0x000000),
            rgb(0xcd3131),
            rgb(0x00bc00),
            rgb(0x949800),
            rgb(0x0451a5),
            rgb(0xbc05bc),
            rgb(0x0598bc),
            rgb(0x555555),
            rgb(0x666666),
            rgb(0xcd3131),
            rgb(0x14ce14),
            rgb(0xb5ba00),
            rgb(0x0451a5),
            rgb(0xbc05bc),
            rgb(0x0598bc),
            rgb(0xa5a5a5),
        ],
    };

    /// The color of a cell. A color that the remote program set wins.
    pub(super) fn resolve(&self, color: Color, set: &Colors) -> Rgb {
        match color {
            Color::Spec(spec) => Rgb([spec.r, spec.g, spec.b]),
            Color::Indexed(index) => set[usize::from(index)].map_or_else(|| self.indexed(index), own),
            Color::Named(name) => set[name].map_or_else(|| self.named(name), own),
        }
    }

    fn named(&self, name: NamedColor) -> Rgb {
        let index = name as usize;
        match name {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground, self.background),
            // The dim colors follow the eight normal colors in the same sequence.
            _ if index >= NamedColor::DimBlack as usize => {
                let normal = index - NamedColor::DimBlack as usize;
                dim(self.ansi.get(normal).copied().unwrap_or(self.foreground), self.background)
            }
            _ => self.ansi.get(index).copied().unwrap_or(self.foreground),
        }
    }

    /// A color of the table of 256.
    fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => self.ansi.get(usize::from(index)).copied().unwrap_or(self.foreground),
            16..=231 => {
                let level = |step: u8| if step == 0 { 0 } else { 55 + 40 * step };
                let cube = index - 16;
                Rgb([level(cube / 36), level(cube / 6 % 6), level(cube % 6)])
            }
            232..=255 => {
                let gray = 8 + 10 * (index - 232);
                Rgb([gray, gray, gray])
            }
        }
    }

    /// The default of a color index, for the answer to a color request.
    pub(super) fn by_index(&self, index: usize) -> ansi::Rgb {
        let Rgb([r, g, b]) = match index {
            0..=255 => self.indexed(u8::try_from(index).unwrap_or(0)),
            257 => self.background,
            258 => self.cursor,
            _ => self.foreground,
        };
        ansi::Rgb { r, g, b }
    }
}

fn own(color: ansi::Rgb) -> Rgb {
    Rgb([color.r, color.g, color.b])
}

/// A color two thirds of the way from the background to `color`: the faint text of SGR 2.
pub(super) fn dim(color: Rgb, background: Rgb) -> Rgb {
    let (Rgb(color), Rgb(background)) = (color, background);
    let mix = |index: usize| {
        let (c, b) = (u16::from(color[index]), u16::from(background[index]));
        u8::try_from((2 * c + b) / 3).unwrap_or(u8::MAX)
    };
    Rgb([mix(0), mix(1), mix(2)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cube_and_the_gray_ramp_are_those_of_xterm() {
        let palette = Palette::DARK;
        assert_eq!(palette.indexed(16), rgb(0x000000));
        assert_eq!(palette.indexed(196), rgb(0xff0000));
        assert_eq!(palette.indexed(208), rgb(0xff8700));
        assert_eq!(palette.indexed(231), rgb(0xffffff));
        assert_eq!(palette.indexed(232), rgb(0x080808));
        assert_eq!(palette.indexed(255), rgb(0xeeeeee));
    }

    #[test]
    fn a_color_that_the_remote_program_set_wins() {
        let mut set = Colors::default();
        set[1] = Some(ansi::Rgb { r: 1, g: 2, b: 3 });
        let palette = Palette::LIGHT;
        assert_eq!(palette.resolve(Color::Named(NamedColor::Red), &set), Rgb([1, 2, 3]));
        assert_eq!(palette.resolve(Color::Indexed(2), &set), rgb(0x00bc00));
        assert_eq!(palette.resolve(Color::Named(NamedColor::DimWhite), &set), dim(rgb(0x555555), palette.background));
        assert_eq!(palette.by_index(257), ansi::Rgb { r: 0xf8, g: 0xf9, b: 0xfb });
    }
}
