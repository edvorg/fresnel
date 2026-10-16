//! A shell in a container of a pod: `vi` and `top`, a paste of 1 MiB, the close of its tab,
//! and an attach to the main process of a container.

use std::time::Instant;

use fresnel_core::app::Command;
use fresnel_core::terminal::{Program, TerminalView};

use crate::bed::{Bed, eventually};
use crate::screen::{alternate, cursor_column, keys, processes};

/// The prompt of the shell of busybox as root. A row of the screen has no space at its end.
const PROMPT: &str = "/ #";

/// Closes the tab of a terminal, as its close button does, and waits until it is gone.
pub(crate) async fn close(bed: &Bed, view: &TerminalView) {
    bed.core.send(Command::CloseTerminal(view.id));
    bed.until("the tab is gone", |now| now.terminals.iter().all(|terminal| terminal.id != view.id)).await;
}

/// Opens a shell in the container `shell` and waits for its prompt.
pub(crate) async fn shell(bed: &Bed) -> TerminalView {
    let view = bed.open(bed.request(Program::Shell, "shell", "shell").await).await;
    bed.screen(&view.terminal, PROMPT).await;
    view
}

/// Plan section 15, M10: `vi` and `top` work. busybox `vi` writes a file with the keys of
/// the user on the alternate screen, and `top` draws its table.
pub(crate) async fn vi_and_top_work_in_a_shell(bed: &Bed) {
    let view = shell(bed).await;
    let terminal = &view.terminal;
    keys(terminal, "echo $TERM\r");
    bed.screen(terminal, "xterm-256color").await;
    keys(terminal, "vi /tmp/notes.txt\r");
    eventually("vi on the alternate screen", || async { alternate(terminal) }).await;
    keys(terminal, "ihello from vi");
    bed.screen(terminal, "hello from vi").await;
    // vi reads Escape and the next key in one read as an escape sequence: the command waits
    // until vi left the insert mode, which moves the cursor back one column.
    let typed = cursor_column(terminal);
    keys(terminal, "\x1b");
    eventually("the end of the insert mode", || async { cursor_column(terminal) < typed }).await;
    keys(terminal, ":wq\r");
    eventually("the shell screen after vi", || async { !alternate(terminal) }).await;
    assert_eq!(bed.kubectl(&["exec", "shell", "--", "cat", "/tmp/notes.txt"]).await, "hello from vi\n");
    keys(terminal, "top\r");
    let rows = bed.screen(terminal, "Mem:").await;
    assert!(rows.iter().any(|row| row.contains("PID") && row.contains("COMMAND")), "{rows:#?}");
    keys(terminal, "q");
    keys(terminal, "echo top-$((1+1))\r");
    bed.screen(terminal, "top-2").await;
    close(bed, &view).await;
}

/// Plan section 15, M10: a paste of 1 MiB arrives complete. The lines carry their numbers,
/// so `awk` finds a line that is missing, doubled or out of its place.
pub(crate) async fn a_paste_of_1_mib_arrives_complete(bed: &Bed) {
    let view = shell(bed).await;
    let terminal = &view.terminal;
    keys(terminal, "stty -echo; echo ready-$((2+2)); cat > /tmp/paste; stty echo\r");
    bed.screen(terminal, "ready-4").await;
    let text: String = (0..16_384).map(|line| format!("{line:08} {}\n", "x".repeat(54))).collect();
    assert_eq!(text.len(), 1024 * 1024);
    let start = Instant::now();
    terminal.paste(&text).expect("the input has room");
    eventually("the session took the paste", || async { terminal.waiting_input() == 0 }).await;
    keys(terminal, "\x04");
    keys(terminal, "echo pasted-$((40+2))\r");
    bed.screen(terminal, "pasted-42").await;
    eprintln!("terminal: a paste of 1 MiB in {} ms", start.elapsed().as_millis());
    let check = "wc -c < /tmp/paste; wc -l < /tmp/paste; awk '$1 != NR-1' /tmp/paste | wc -l";
    let counts = bed.kubectl(&["exec", "shell", "--", "sh", "-c", check]).await;
    let counts: Vec<&str> = counts.split_whitespace().collect();
    assert_eq!(counts, ["1048576", "16384", "0"], "the bytes, the lines, and the lines out of place");
    close(bed, &view).await;
}

/// Plan section 15, M10: a closed tab leaves no process in the pod. Ctrl-D does not end
/// `vi`, and two jobs run in the background, one of them with `nohup`. The path makes the
/// shell of busybox start `sleep` as a program: else it runs it in a copy of itself.
pub(crate) async fn a_closed_tab_leaves_no_process_in_the_pod(bed: &Bed) {
    let view = shell(bed).await;
    let terminal = &view.terminal;
    keys(terminal, "/bin/sleep 1001 & nohup sleep 1002 > /dev/null 2>&1 & echo started-$((3*3))\r");
    bed.screen(terminal, "started-9").await;
    keys(terminal, "vi /tmp/open.txt\r");
    eventually("vi on the alternate screen", || async { alternate(terminal) }).await;
    let before = processes(bed, "shell", "shell").await;
    for program in ["sleep 1001", "sleep 1002", "vi /tmp/open.txt"] {
        assert!(before.iter().any(|line| line.ends_with(program)), "{program} runs: {before:#?}");
    }
    close(bed, &view).await;
    no_session_process(bed, "shell").await;
    let journal = bed.core.view().journal.clone();
    assert!(journal.entries.iter().any(|entry| entry.text.contains("a hangup went to")), "the journal tells it");
}

/// An attach reaches the main process of a container, and the close of its tab leaves it.
pub(crate) async fn an_attach_reaches_the_main_process_and_its_close_leaves_it(bed: &Bed) {
    let view = bed.open(bed.request(Program::Attach, "main", "main").await).await;
    keys(&view.terminal, "echo attached-$((5*5))\r");
    bed.screen(&view.terminal, "attached-25").await;
    close(bed, &view).await;
    // A second attach still reaches the same shell.
    let again = bed.open(bed.request(Program::Attach, "main", "main").await).await;
    keys(&again.terminal, "echo again-$((6*6))\r");
    bed.screen(&again.terminal, "again-36").await;
    close(bed, &again).await;
    let phase = bed.kubectl(&["get", "pod", "main", "-o", "jsonpath={.status.phase}"]).await;
    assert_eq!(phase, "Running", "the close of an attach leaves the main process");
}

/// Waits until no process of a session runs in the container `shell`. Its PID 1 is a reaper,
/// so a process that the signal left would run; a zombie (`[name]`) cannot stay, and a leak
/// of a session shows as a live `sleep` or `vi`. The reaper and its own `sleep 1` are not
/// session processes.
pub(crate) async fn no_session_process(bed: &Bed, pod: &str) {
    let leaked = |now: &[String]| {
        now.iter().any(|line| {
            let line = line.trim_start();
            line.contains('[')
                || line.contains("sleep 100")
                || line.contains("sleep 1003")
                || line.ends_with("vi /tmp/open.txt")
        })
    };
    let wait = async {
        while leaked(&processes(bed, pod, "shell").await) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    if tokio::time::timeout(crate::bed::LIMIT, wait).await.is_err() {
        let left = processes(bed, pod, "shell").await;
        let journal: Vec<String> =
            bed.core.view().journal.entries.iter().map(|entry| crate::bed::line(entry)).collect();
        panic!("in time: no process of a session in the pod\n{left:#?}\n{journal:#?}");
    }
}
