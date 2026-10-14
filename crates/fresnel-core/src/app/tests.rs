//! The tests of the core itself: its journal and its queue of commands.

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
