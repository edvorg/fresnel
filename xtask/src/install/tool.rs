//! A run of a tool of the system, for example `codesign`.

use std::ffi::OsStr;
use std::io;
use std::process::{Command, Stdio};

/// Runs a tool to its end. A status that is not zero is an error with the text of the tool.
pub(super) fn run<I, S>(tool: &str, arguments: I) -> io::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(tool).args(arguments).stdin(Stdio::null()).output();
    let output = output.map_err(|error| io::Error::new(error.kind(), format!("cannot start {tool}: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    Err(io::Error::other(format!("{tool} failed ({}): {}", output.status, text.trim())))
}
