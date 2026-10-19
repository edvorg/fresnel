//! The differences of the systems in the core: the name of the command key in the text of a
//! shortcut. Only macOS has a Command key. On Windows, and on Linux for CI, the command key is
//! Control, so its text says `Ctrl`, and `Ctrl` in a text names the same key as `Cmd`.

/// The name of the command key in the text of a shortcut, as `Cmd+K`.
#[cfg(target_os = "macos")]
pub const COMMAND_KEY: &str = "Cmd";
/// The name of the command key in the text of a shortcut, as `Ctrl+K`.
#[cfg(not(target_os = "macos"))]
pub const COMMAND_KEY: &str = "Ctrl";

/// True where Control is the command key: then `Ctrl` in a text is the command key, so one
/// key has one text and one shortcut.
pub(crate) const CONTROL_IS_COMMAND: bool = cfg!(not(target_os = "macos"));
