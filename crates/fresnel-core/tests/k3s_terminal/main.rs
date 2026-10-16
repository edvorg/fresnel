//! The terminal of the app core on k3s in Docker behind the fault proxy (plan section 15,
//! M10): each command goes the way of the window, from `Core::send` to the view of the core,
//! and kubectl reads the pod.
//!
//! - `bed`: the cluster, its pods and the core. `screen`: the rows of a screen and the keys.
//! - `shell`: a shell with `vi` and `top`, a paste of 1 MiB, the close of a tab, an exit, an
//!   attach.
//! - `made`: a debug container and a node shell, which the app makes and ends.
//! - `silence`: a silent connection, and the close of the tab of its failed session.

// k3s in Docker: the cluster job of CI runs on Linux.
#![cfg(unix)]
#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod bed;
#[path = "../common/contexts.rs"]
mod contexts;
mod made;
mod screen;
mod shell;
mod silence;

use bed::Bed;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn the_terminal_runs_vi_and_top_takes_a_paste_and_leaves_no_process_after_its_close() {
    let bed = Bed::start().await;
    shell::vi_and_top_work_in_a_shell(&bed).await;
    shell::a_paste_of_1_mib_arrives_complete(&bed).await;
    shell::a_closed_tab_leaves_no_process_in_the_pod(&bed).await;
    shell::an_exit_code_decides_the_final_state_and_no_hangup_goes(&bed).await;
    shell::a_close_during_a_paste_into_a_program_that_reads_nothing_leaves_no_process(&bed).await;
    shell::an_attach_reaches_the_main_process_and_its_close_leaves_it(&bed).await;
    shell::an_attach_to_a_container_with_no_input_refuses_the_keys(&bed).await;
    made::a_debug_container_sees_the_processes_of_its_target_and_ends_with_its_tab(&bed).await;
    made::a_node_shell_runs_on_the_node_and_its_pod_goes_with_its_tab(&bed).await;
    // The silence comes last: it breaks each connection of the cluster.
    silence::a_silent_exec_connection_fails_in_20_s_and_its_close_ends_the_shell(&bed).await;
}
