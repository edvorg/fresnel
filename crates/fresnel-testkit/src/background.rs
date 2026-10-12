//! The background tasks of one kit part.
//!
//! The kit starts its tasks with the start function of the app (design rule 9). A kit task
//! that ends is a fault of the test equipment. Each part keeps the reports of its tasks, so
//! that a test can show them.

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, TaskReport, Tasks};
use tokio::runtime::Handle;
use tokio::task::JoinError;

use crate::KitError;

/// The first reports are the useful ones. A later report most often repeats the first.
const KEPT_REPORTS: usize = 8;

/// The tasks of one kit part and the reports of their ends.
pub(crate) struct Background {
    tasks: Tasks,
    ends: Arc<Mutex<Ends>>,
}

#[derive(Default)]
struct Ends {
    first: Vec<String>,
    later: u64,
}

impl Background {
    /// The tasks run on the runtime of the caller.
    pub(crate) fn new() -> Result<Self, KitError> {
        let runtime =
            Handle::try_current().map_err(|error| KitError::because("the test kit needs a runtime", error))?;
        let ends = Arc::new(Mutex::new(Ends::default()));
        let reporter = {
            let ends = Arc::clone(&ends);
            move |report: TaskReport| {
                // A test that stops at a time limit never reads the list. Its output shows this line.
                eprintln!("fresnel-testkit: {report}");
                let mut ends = ends.lock().unwrap_or_else(PoisonError::into_inner);
                if ends.first.len() < KEPT_REPORTS {
                    ends.first.push(report.to_string());
                } else {
                    ends.later += 1;
                }
            }
        };
        Ok(Self { tasks: Tasks::new(runtime, reporter), ends })
    }

    /// Starts a task that must not end. If it ends, it leaves a report and runs again.
    pub(crate) fn start<B, F>(&self, role: &'static str, body: B) -> TaskHandle
    where
        B: FnMut() -> F + Send + 'static,
        F: Future<Output = io::Result<()>> + Send + 'static,
    {
        self.tasks.start(TaskName::new(role), Restart::Always(Backoff::default()), body)
    }

    /// One line for each task run that ended. An empty list is the normal case.
    pub(crate) fn failures(&self) -> Vec<String> {
        let ends = self.ends.lock().unwrap_or_else(PoisonError::into_inner);
        let mut lines = ends.first.clone();
        if ends.later > 0 {
            lines.push(format!("and {} more", ends.later));
        }
        lines
    }
}

/// Reads the end of one connection task. A panic there is an error of the task that owns the
/// connections, so it gets a report. An abort is the normal end after a reset.
pub(crate) fn connection_end(ended: Result<(), JoinError>) -> io::Result<()> {
    match ended {
        Err(error) if error.is_panic() => Err(io::Error::other(format!("a connection task failed: {error}"))),
        Ok(()) | Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::{Duration, sleep};

    use super::*;

    /// Paused time moves to the next pause of the backoff when the test waits.
    #[tokio::test(start_paused = true)]
    async fn a_task_that_ends_leaves_a_report_and_the_list_has_a_limit() {
        let background = Background::new().expect("a runtime");
        let _task = background.start("accept", || async { Err(io::Error::other("no more files")) });
        assert_eq!(background.failures(), Vec::<String>::new());

        sleep(Duration::from_millis(1)).await;
        let failures = background.failures();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].starts_with("task accept failed: no more files; restart in "), "{failures:?}");

        while background.failures().len() <= KEPT_REPORTS {
            sleep(Duration::from_millis(100)).await;
        }
        let failures = background.failures();
        assert_eq!(failures.len(), KEPT_REPORTS + 1);
        assert_eq!(failures.last().map(String::as_str), Some("and 1 more"));
    }

    #[tokio::test]
    async fn a_panic_of_a_connection_is_an_error_and_an_abort_is_not() {
        let mut connections = tokio::task::JoinSet::new();
        connections.spawn(async { panic!("bad state") });
        let panicked = connections.join_next().await.expect("one task");
        assert!(connection_end(panicked).expect_err("a panic").to_string().starts_with("a connection task failed"));

        connections.spawn(std::future::pending::<()>());
        connections.abort_all();
        let aborted = connections.join_next().await.expect("one task");
        assert!(connection_end(aborted).is_ok());
        assert!(connection_end(Ok(())).is_ok());
    }

    #[test]
    fn a_part_that_starts_with_no_runtime_gives_an_error() {
        let error = Background::new().err().expect("no runtime here").to_string();
        assert!(error.starts_with("the test kit needs a runtime"), "{error}");
    }
}
