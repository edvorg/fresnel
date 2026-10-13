//! The runtime of one command.

use std::process::ExitCode;
use std::time::Duration;

use tokio::runtime::{Builder, Handle, Runtime};

/// The longest wait for the tasks of the runtime at the end of the command. The tasks of a
/// cluster end at once, and a task ends the process of its plugin.
const EXIT_LIMIT: Duration = Duration::from_secs(1);

/// Runs a command on a new runtime and gives its exit status.
pub(crate) fn run<F: Future<Output = ExitCode>>(command: impl FnOnce(Handle) -> F) -> ExitCode {
    let runtime = match Builder::new_multi_thread().worker_threads(2).enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let status = runtime.block_on(command(runtime.handle().clone()));
    stop(runtime);
    status
}

/// Stops the runtime at the end of the command.
///
/// The drop of a runtime waits for each blocking call with no limit. An address lookup of
/// the system is such a call: it cannot be stopped, and it can need 30 seconds on a network
/// with no name server. The command has its report then, so it does not wait for the call.
fn stop(runtime: Runtime) {
    runtime.shutdown_timeout(EXIT_LIMIT);
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn the_end_of_the_command_does_not_wait_for_a_blocking_call() {
        let runtime = Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a runtime");
        let (started, runs) = std::sync::mpsc::channel();
        let call = runtime.spawn_blocking(move || {
            started.send(()).expect("the test waits for this");
            std::thread::sleep(4 * EXIT_LIMIT);
        });
        runs.recv().expect("the blocking call runs");
        let stopped = Instant::now();
        stop(runtime);
        assert!(stopped.elapsed() < 2 * EXIT_LIMIT, "the stop took {:?}", stopped.elapsed());
        drop(call);
    }
}
