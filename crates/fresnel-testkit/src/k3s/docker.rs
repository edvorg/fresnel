//! Runs `docker` commands. The test kit starts no other program.

use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::time::timeout;

use crate::KitError;

/// The limit for one command. The first start of a container can pull the image.
const LIMIT: Duration = Duration::from_secs(300);

/// Runs one command and gives its standard output. A status that is not 0 is an error with
/// the error text of docker.
pub(super) async fn docker(arguments: &[&str]) -> Result<String, KitError> {
    let mut command = tokio::process::Command::new("docker");
    command.args(arguments).stdin(Stdio::null()).kill_on_drop(true);
    let output = timeout(LIMIT, command.output())
        .await
        .map_err(|_| KitError::new(format!("docker {}: no result in {LIMIT:?}", arguments.join(" "))))?;
    read(arguments, output)
}

/// The same for `Drop`, which cannot wait for a future.
pub(super) fn docker_blocking(arguments: &[&str]) -> Result<String, KitError> {
    let output = std::process::Command::new("docker").args(arguments).stdin(Stdio::null()).output();
    read(arguments, output)
}

fn read(arguments: &[&str], output: std::io::Result<Output>) -> Result<String, KitError> {
    let command = format!("docker {}", arguments.join(" "));
    let output = output.map_err(|error| KitError::because(format!("{command}: cannot start"), error))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(KitError::because(command, String::from_utf8_lossy(&output.stderr).trim()))
    }
}
