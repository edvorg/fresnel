//! A silent connection under a shell: the session fails in its limit, the line tells that
//! the shell can still run, and the close of the tab ends it through a new connection.

use std::time::{Duration, Instant};

use fresnel_core::terminal::SessionState;

use crate::bed::Bed;
use crate::screen::{keys, processes};
use crate::shell::{close, no_session_process, shell};

/// Plan section 15, M10: a silent exec connection shows as failed in 20 s or less.
const SILENCE_LIMIT: Duration = Duration::from_secs(20);

pub(crate) async fn a_silent_exec_connection_fails_in_20_s_and_its_close_ends_the_shell(bed: &Bed) {
    let view = shell(bed).await;
    let terminal = &view.terminal;
    keys(terminal, "/bin/sleep 1003 & echo quiet-$((6*7))\r");
    bed.screen(terminal, "quiet-42").await;
    let silenced = Instant::now();
    bed.proxy.silence_open();
    bed.until("the failed session", |_| matches!(terminal.state(), SessionState::Failed { .. })).await;
    let elapsed = silenced.elapsed();
    eprintln!("terminal: a silent connection failed after {} ms", elapsed.as_millis());
    assert!(elapsed <= SILENCE_LIMIT, "the silence shows in {elapsed:?}");
    let SessionState::Failed { reason } = terminal.state() else { unreachable!("the state failed") };
    assert!(reason.starts_with("silent: no frame for"), "{reason}");
    assert!(reason.ends_with("the shell can still run in the pod: a close of the tab ends it"), "{reason}");
    let running = processes(bed, "shell", "shell").await;
    assert!(running.iter().any(|line| line.ends_with("sleep 1003")), "the shell runs on: {running:#?}");
    close(bed, &view).await;
    no_session_process(bed, "shell").await;
}
