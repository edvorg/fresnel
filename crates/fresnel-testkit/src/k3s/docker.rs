//! Runs `docker` commands. The test kit starts no other program.

use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::time::timeout;

use crate::KitError;

/// The limit for one command. The first start of a container can pull the image.
const LIMIT: Duration = Duration::from_secs(300);
/// The tries of a removal in a drop, and the pause between two: Docker refuses to remove a
/// network while the removal of its container still runs.
const REMOVE_TRIES: u32 = 3;
const REMOVE_PAUSE: Duration = Duration::from_millis(500);

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
fn docker_blocking(arguments: &[&str]) -> Result<String, KitError> {
    let output = std::process::Command::new("docker").args(arguments).stdin(Stdio::null()).output();
    read(arguments, output)
}

/// Removes a container or a network in a drop, with these arguments of `docker`. One that
/// stays after each try fails the test with a panic, unless a panic runs already: it would
/// hold memory and CPU of the Docker VM for each later test, and a line of the output is easy
/// to miss. One that is gone already counts as removed.
pub(super) fn remove_in_drop(arguments: &[&str]) {
    fail_loud(tries(|| docker_blocking(arguments), REMOVE_PAUSE));
}

fn fail_loud(removed: Result<(), KitError>) {
    if let Err(error) = removed {
        let text = format!("fresnel-testkit: {error}");
        if std::thread::panicking() {
            eprintln!("{text}");
        } else {
            panic!("{text}");
        }
    }
}

/// Runs a removal until it works or the thing is gone, [`REMOVE_TRIES`] times at most.
fn tries(mut remove: impl FnMut() -> Result<String, KitError>, pause: Duration) -> Result<(), KitError> {
    let mut tried = 1;
    loop {
        match remove() {
            Ok(_) => return Ok(()),
            // `docker rm --force` takes a container that is gone; `docker network rm` does not.
            Err(error) if error.to_string().contains("not found") => return Ok(()),
            Err(error) if tried == REMOVE_TRIES => return Err(KitError::because(format!("{tried} tries"), error)),
            Err(_) => {}
        }
        tried += 1;
        std::thread::sleep(pause);
    }
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// Runs the tries on removals that fail with these texts, one for each try, and then work.
    fn tried(errors: &[&str]) -> (Result<(), String>, usize) {
        let calls = Cell::new(0);
        let remove = || {
            let call = calls.replace(calls.get() + 1);
            errors.get(call).map_or_else(|| Ok(String::new()), |text| Err(KitError::new(*text)))
        };
        (tries(remove, Duration::ZERO).map_err(|error| error.to_string()), calls.get())
    }

    #[test]
    fn a_removal_tries_again_and_a_thing_that_is_gone_counts_as_removed() {
        assert_eq!(tried(&[]), (Ok(()), 1));
        assert_eq!(tried(&["has active endpoints", "has active endpoints"]), (Ok(()), 3));
        assert_eq!(tried(&["network fresnel-test-a not found"]), (Ok(()), 1));
        let busy = ["busy"; 3];
        assert_eq!(tried(&busy), (Err("3 tries: busy".to_owned()), 3));
    }

    /// A test that leaves a container or a network fails, not only a line of its output.
    #[test]
    #[should_panic(expected = "fresnel-testkit: 3 tries: docker rm: busy")]
    fn a_container_that_stays_fails_the_test() {
        fail_loud(Err(KitError::new("3 tries: docker rm: busy")));
    }
}
