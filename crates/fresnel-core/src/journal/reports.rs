//! The journal as the receiver of each task report (design rule 1).
//!
//! `fresnel_net::task::Tasks` gives one report for each run of a task that ended. With the
//! journal as its reporter, no task of the app and no worker thread of the UI can end with
//! no entry.

use fresnel_net::task::{End, Reporter, TaskReport};

use super::{Journal, Level};

/// The source of the entry for a task run that ended.
pub const TASK_SOURCE: &str = "task";

impl Reporter for Journal {
    fn report(&self, report: TaskReport) {
        self.push(level_of(&report), TASK_SOURCE, report.to_string());
    }
}

/// A fault is an error. A task that restarts must not end, so its clean end is a warning,
/// and so is a task with no runtime. Only the clean end of a job is an event.
fn level_of(report: &TaskReport) -> Level {
    match (&report.end, report.restart_in) {
        (End::Failed(_) | End::Panicked(_), _) => Level::Error,
        (End::Done, Some(_)) | (End::Stopped, _) => Level::Warning,
        (End::Done, None) => Level::Info,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use fresnel_net::WallClock;
    use fresnel_net::task::{Backoff, Restart, Stop, TaskName, Tasks};
    use tokio::runtime::Handle;
    use tokio::time::sleep;

    use super::*;
    use crate::error::CoreError;

    fn entries(journal: &Journal) -> Vec<(Level, String, String)> {
        let view = journal.view();
        view.entries.iter().map(|entry| (entry.level, entry.source.clone(), entry.text.clone())).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn each_run_of_a_task_that_ends_is_an_entry_with_its_level() {
        let journal = Journal::new(100, WallClock::system());
        let tasks = Tasks::new(Handle::current(), journal.clone());
        let ends = |fails: bool| move || async move { if fails { Err(CoreError::invalid("no luck")) } else { Ok(()) } };
        let job = tasks.start(TaskName::new("job"), Restart::Never, ends(false));
        let broken = tasks.start(TaskName::new("broken job"), Restart::Never, ends(true));
        let pause = Backoff { first: Duration::from_secs(600), ..Backoff::default() };
        let service = tasks.start(TaskName::new("service"), Restart::Always(pause), ends(false));
        sleep(Duration::from_secs(1)).await;

        let mut entries = entries(&journal);
        entries.sort();
        assert_eq!(entries.len(), 3, "{entries:?}");
        let task = TASK_SOURCE.to_owned();
        assert_eq!(entries[0], (Level::Info, task.clone(), "task job ended; no restart".to_owned()));
        assert_eq!(entries[1].0, Level::Warning, "a task that must stay alive ended: {entries:?}");
        assert!(entries[1].2.starts_with("task service ended; restart in "), "{entries:?}");
        assert_eq!(entries[2], (Level::Error, task, "task broken job failed: no luck; no restart".to_owned()));
        drop((job, broken, service));
    }

    /// Plan section 13.5 for a worker thread: a panic gives a journal entry and a restart.
    #[tokio::test]
    async fn a_panic_of_a_worker_thread_is_an_error_entry_and_the_thread_runs_again() {
        let journal = Journal::new(100, WallClock::system());
        let tasks = Tasks::new(Handle::current(), journal.clone());
        let (run_sender, runs) = channel();
        let pause = Backoff { first: Duration::from_millis(1), ..Backoff::default() };
        let mut run = 0;
        let worker = tasks.start_thread(TaskName::scoped("rows", "ams"), Restart::Always(pause), move |stop: &Stop| {
            run += 1;
            assert_ne!(run, 1, "a defect in the worker");
            run_sender.send(run).map_err(|_| CoreError::invalid("the test is gone"))?;
            while !stop.is_set() {
                std::thread::yield_now();
            }
            Ok::<(), CoreError>(())
        });

        assert_eq!(runs.recv_timeout(Duration::from_secs(30)), Ok(2), "the second run of the worker");
        let entries = entries(&journal);
        assert_eq!(entries.len(), 1, "{entries:?}");
        let (level, source, text) = &entries[0];
        assert_eq!((*level, source.as_str()), (Level::Error, TASK_SOURCE));
        assert!(text.starts_with("task rows [ams] panicked: assertion `left != right` failed: a defect"), "{text}");
        assert!(text.contains("; restart in "), "{text}");
        drop(worker);
    }
}
