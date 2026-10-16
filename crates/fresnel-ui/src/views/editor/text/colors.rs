//! The syntax colors of the editor, one set for each theme: a color that reads well on a
//! dark background is too light on a light one. The values are the colors of YAML in the
//! dark and the light theme of GitHub, which users of code know.

use egui::Color32;

use super::lexer::Kind;
use crate::theme::Palette;

/// The colors of the kinds of a line that are not the text color.
struct Syntax {
    key: Color32,
    quoted: Color32,
    number: Color32,
    word: Color32,
    tag: Color32,
}

const DARK: Syntax = Syntax {
    key: Color32::from_rgb(0x7e, 0xe7, 0x87),
    quoted: Color32::from_rgb(0xa5, 0xd6, 0xff),
    number: Color32::from_rgb(0x79, 0xc0, 0xff),
    word: Color32::from_rgb(0xd2, 0xa8, 0xff),
    tag: Color32::from_rgb(0xff, 0xa6, 0x57),
};

const LIGHT: Syntax = Syntax {
    key: Color32::from_rgb(0x11, 0x63, 0x29),
    quoted: Color32::from_rgb(0x0a, 0x30, 0x69),
    number: Color32::from_rgb(0x05, 0x50, 0xae),
    word: Color32::from_rgb(0x82, 0x50, 0xdf),
    tag: Color32::from_rgb(0x95, 0x38, 0x00),
};

/// The color of a kind in a theme.
pub(crate) fn of(kind: Kind, palette: &Palette, dark: bool) -> Color32 {
    let syntax = if dark { &DARK } else { &LIGHT };
    match kind {
        Kind::Key => syntax.key,
        Kind::Quoted | Kind::Block => syntax.quoted,
        Kind::Number => syntax.number,
        Kind::Word => syntax.word,
        Kind::Tag => syntax.tag,
        Kind::Comment => palette.text_faint,
        Kind::Mark | Kind::Document => palette.text_muted,
        Kind::Text => palette.text,
    }
}
