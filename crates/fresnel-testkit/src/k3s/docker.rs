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
    read(arguments, output(arguments, None).await?)
}

/// Runs one command with a secret in the environment of docker, which `--env NAME` in the
/// arguments takes from there: no argument holds the value, so no error text shows it.
pub(super) async fn docker_with_secret(arguments: &[&str], secret: (&str, &str)) -> Result<String, KitError> {
    read(arguments, output(arguments, Some(secret)).await?)
}

/// Runs one command and gives its standard output and its error output as one text, for
/// each exit status. `docker logs` gives the lines of k3s on the error output.
pub(super) async fn docker_with_errors(arguments: &[&str]) -> Result<String, KitError> {
    let output = output(arguments, None).await?;
    let output =
        output.map_err(|error| KitError::because(format!("docker {}: cannot start", arguments.join(" ")), error))?;
    Ok(format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
}

/// Runs one command to its end, with the time limit.
async fn output(arguments: &[&str], secret: Option<(&str, &str)>) -> Result<std::io::Result<Output>, KitError> {
    let mut command = tokio::process::Command::new("docker");
    command.args(arguments).envs(secret).stdin(Stdio::null()).kill_on_drop(true);
    let limited = timeout(LIMIT, command.output()).await;
    limited.map_err(|_| KitError::new(format!("docker {}: no result in {LIMIT:?}", arguments.join(" "))))
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
