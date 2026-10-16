//! The fonts of macOS.

use super::SystemFont;

/// Menlo Bold. Menlo and Hack, the monospace font of egui, both come from Bitstream Vera:
/// a bold run has the advance of a regular one.
pub(crate) fn bold_font() -> Option<SystemFont> {
    Some(SystemFont { path: "/System/Library/Fonts/Menlo.ttc".into(), index: 1 })
}

/// Hiragino Sans GB, with Chinese and the Japanese kana: the smallest CJK font of the system,
/// 23 MB.
pub(crate) fn wide_font() -> Option<SystemFont> {
    Some(SystemFont { path: "/System/Library/Fonts/Hiragino Sans GB.ttc".into(), index: 0 })
}

#[cfg(test)]
mod tests {
    /// A wrong path costs the bold face or the CJK glyphs, with only a line in the journal.
    #[test]
    fn the_font_files_are_on_the_system() {
        for font in [super::bold_font(), super::wide_font()].into_iter().flatten() {
            assert!(font.path.is_file(), "{}", font.path.display());
        }
    }
}
