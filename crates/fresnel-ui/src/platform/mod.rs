//! The differences of the systems in the views: the font files of the system for the
//! terminal. Each one has a variant for macOS, for Windows and for the other systems (Linux,
//! for CI) with the same names, so the code outside this module has no `cfg` of a system.

use std::path::PathBuf;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod system;
#[cfg(windows)]
#[path = "windows.rs"]
mod system;
#[cfg(not(any(target_os = "macos", windows)))]
#[path = "other.rs"]
mod system;

pub(crate) use self::system::{bold_font, wide_font};

/// A font file of the system, and the index of the face in a collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SystemFont {
    pub(crate) path: PathBuf,
    pub(crate) index: u32,
}
