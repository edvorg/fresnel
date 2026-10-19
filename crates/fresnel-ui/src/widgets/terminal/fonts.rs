//! The fonts of the terminal: a regular and a bold family of egui, which start as the
//! monospace fonts of egui with a font of symbols after the first one.
//!
//! egui has no bold flag and its fonts have no CJK glyphs: `中文` showed as two boxes in the
//! spike. The program reads the bold face and the font for wide characters of its system
//! ([`TerminalFonts::of_system`], `platform`) and adds them ([`add_fonts`]); a view test has
//! neither, and a bold run is then painted two times.
//!
//! The fonts of egui have no `✓` or `✗`, which test tools print, and no control pictures. The
//! app carries JetBrains Mono 2.304 Regular (SIL Open Font License 1.1, `fonts/OFL.txt`, 274 KB),
//! a monospace font with 335 such code points; each one is painted alone in its cell.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{Context, FontData, FontDefinitions, FontFamily, FontId};

use crate::platform::{self, SystemFont};

const REGULAR: &str = "terminal";
const BOLD: &str = "terminal bold";
/// The names of the faces in the fonts of egui.
const BOLD_FACE: &str = "terminal bold face";
const WIDE_FACE: &str = "terminal wide face";
const SYMBOLS_FACE: &str = "terminal symbols";
/// The font of the symbols that the monospace font of egui has not.
const SYMBOLS: &[u8] = include_bytes!("../../../fonts/JetBrainsMono-Regular.ttf");

/// A font file: its bytes, and the index of the face in a collection.
#[derive(Debug)]
struct FontFile {
    bytes: Vec<u8>,
    index: u32,
}

/// The fonts of the system for the terminal.
#[derive(Debug)]
pub struct TerminalFonts {
    /// A bold face with the advance of the monospace font of egui (Menlo Bold on macOS).
    bold: Option<FontFile>,
    /// A font for the characters that the monospace font has not, such as CJK.
    wide: Option<FontFile>,
}

impl TerminalFonts {
    /// Reads the font files of the system: 25 MB on macOS, in a few milliseconds. Gives the
    /// fonts and a text for each file that the system should have and did not give.
    pub fn of_system() -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut read = |font: Option<SystemFont>, role: &str| {
            let font = font?;
            match std::fs::read(&font.path) {
                Ok(bytes) => Some(FontFile { bytes, index: font.index }),
                Err(error) => {
                    problems.push(format!("the terminal has no {role} font: {}: {error}", font.path.display()));
                    None
                }
            }
        };
        let fonts = Self { bold: read(platform::bold_font(), "bold"), wide: read(platform::wide_font(), "CJK") };
        (fonts, problems)
    }
}

/// The two families with the monospace fonts of egui, and the font of symbols after the first
/// one, so a symbol takes a monospace face before a proportional one. `Shell::new` calls it.
pub(crate) fn install(ctx: &Context) {
    let mut definitions = FontDefinitions::default();
    definitions.font_data.insert(SYMBOLS_FACE.to_owned(), FontData::from_static(SYMBOLS).into());
    let mut monospace = definitions.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    monospace.insert(monospace.len().min(1), SYMBOLS_FACE.to_owned());
    definitions.families.insert(FontFamily::Name(REGULAR.into()), monospace.clone());
    definitions.families.insert(FontFamily::Name(BOLD.into()), monospace);
    ctx.set_fonts(definitions);
}

/// Adds the fonts of the system to the families of the terminal: the bold face first in the
/// bold family, the font of wide characters last in both. Call it after `Shell::new`.
pub fn add_fonts(ctx: &Context, fonts: TerminalFonts) {
    let family = |name: &str, priority| InsertFontFamily { family: FontFamily::Name(name.into()), priority };
    let data = |file: FontFile| FontData { index: file.index, ..FontData::from_owned(file.bytes) };
    if let Some(file) = fonts.bold {
        ctx.add_font(FontInsert::new(BOLD_FACE, data(file), vec![family(BOLD, FontPriority::Highest)]));
    }
    if let Some(file) = fonts.wide {
        let families = vec![family(REGULAR, FontPriority::Lowest), family(BOLD, FontPriority::Lowest)];
        ctx.add_font(FontInsert::new(WIDE_FACE, data(file), families));
    }
}

/// The fonts of one size.
pub(super) struct Faces {
    pub(super) regular: FontId,
    pub(super) bold: FontId,
    /// False when the bold family has no bold face.
    pub(super) bold_face: bool,
}

pub(super) fn faces(ctx: &Context, size: f32) -> Faces {
    let bold_face = ctx.fonts(|fonts| fonts.definitions().font_data.contains_key(BOLD_FACE));
    Faces {
        regular: FontId::new(size, FontFamily::Name(REGULAR.into())),
        bold: FontId::new(size, FontFamily::Name(BOLD.into())),
        bold_face,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The monospace fonts of egui have no `✓`, which showed as a box: the font of symbols
    /// gives it and the other marks of test tools, in both families of the terminal.
    #[test]
    fn the_families_of_the_terminal_have_the_marks_of_test_tools() {
        let ctx = Context::default();
        install(&ctx);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let (marks, faces) = ("✓✗✕⏎␀", faces(&ctx, 13.0));
        ctx.fonts_mut(|fonts| {
            assert!(!fonts.has_glyphs(&FontId::monospace(13.0), marks), "the monospace fonts of egui have none");
            assert!(fonts.has_glyphs(&faces.regular, marks) && fonts.has_glyphs(&faces.bold, marks));
        });
    }
}
