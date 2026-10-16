//! The fonts of Windows.

use std::path::PathBuf;

use super::SystemFont;

/// None: Consolas Bold has an other advance and other shapes than Hack, the monospace font of
/// egui. A bold run is painted two times instead.
pub(crate) fn bold_font() -> Option<SystemFont> {
    None
}

/// Microsoft YaHei, with Chinese and the Japanese kana, in the font folder of Windows.
pub(crate) fn wide_font() -> Option<SystemFont> {
    let windows = std::env::var_os("WINDIR")?;
    Some(SystemFont { path: PathBuf::from(windows).join("Fonts").join("msyh.ttc"), index: 0 })
}
