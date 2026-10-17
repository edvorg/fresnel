//! The tests of the core itself: its journal and its queue of commands.

use std::time::Duration;

use fresnel_net::WallClock;
use tokio::time::sleep;

use super::*;
use crate::error::CoreError;
use crate::journal::Level;

fn core() -> Core {
    Core::new(CoreParts {
        runtime: Handle::current(),
        base_env: ProcessEnv::default(),
        journal: Journal::new(100, WallClock::system()),
        // No test of this module starts the core, so nothing reads or writes the files.
        state_file: PathBuf::from("/no/such/folder/state.json"),
        config_file: PathBuf::from("/no/such/folder/config.toml"),
        downloads: PathBuf::from("/no/such/folder/Downloads"),
        detectors: Vec::new(),
        settings: CoreSettings::default(),
    })
}

fn journal(core: &Core) -> Vec<(Level, String)> {
    core.view().journal.entries.iter().map(|entry| (entry.level, entry.text.clone())).collect()
}

/// Design rule 1: the journal of the core is the receiver of the reports of its tasks.
#[tokio::test(start_paused = true)]
async fn a_task_of_the_core_that_ends_is_in_the_journal() {
    let core = core();
    let fails = || async { Err(CoreError::invalid("no luck")) };
    let broken = core.inner.tasks.start(TaskName::new("broken job"), Restart::Never, fails);
    sleep(Duration::from_secs(1)).await;
    assert_eq!(journal(&core), [(Level::Error, "task broken job failed: no luck; no restart".to_owned())]);
    drop(broken);
}

#[tokio::test]
async fn the_commands_that_wait_have_a_limit_and_a_lost_command_is_in_the_journal() {
    let core = core();
    let command = || Command::RetryNow(ClusterId::new("ams"));
    (0..COMMAND_LIMIT).for_each(|_| core.send(command()));
    assert_eq!(journal(&core), []);
    core.send(command());
    assert_eq!(journal(&core), [(Level::Error, "a command is lost: 256 commands wait for the core".to_owned())]);
    assert_eq!(core.inner.state().commands.len(), COMMAND_LIMIT);

    // A stopped core takes no command: no task reads it.
    core.stop();
    core.send(command());
    let waiting = core.inner.state().commands.len();
    assert_eq!((waiting, journal(&core).len()), (0, 1));
}

#[tokio::test]
async fn the_memory_account_names_the_journal_the_tasks_and_the_commands_of_a_core_with_no_cluster() {
    let core = core();
    core.inner.journal.info(APP, "one entry");
    (0..3).for_each(|_| core.send(Command::RetryNow(ClusterId::new("ams"))));
    let parts = core.memory(&crate::memory::NoScale);
    let items = |name: &str| parts.iter().find(|part| part.name == name).map(|part| part.items);
    assert_eq!((items("journal"), items("commands"), items("jobs")), (Some(1), Some(3), Some(0)));
    let journal = parts.iter().find(|part| part.name == "journal").map_or(0, |part| part.bytes);
    assert!(journal >= "one entry".len() + APP.len(), "{journal}");
    assert!(parts.iter().all(|part| !part.name.contains(": ")), "no cluster: {parts:?}");
}
