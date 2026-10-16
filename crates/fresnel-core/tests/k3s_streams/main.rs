//! Exec, attach and port forward on a real kube-apiserver (plan sections 12.3 and 12.8): k3s
//! in Docker behind the fault proxy, on the connections of the app, with short pings.
//!
//! - `bed`: the cluster, its two pods and the connections of the app.
//! - `exec`: the channels, the exit codes, the size of a terminal and the refusals.
//! - `forward`: a tunnel to a small web server, and to a port with no server.
//! - `faults`: a pod that reads no input, a silent connection and a reset.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod bed;
#[path = "../common/mod.rs"]
mod common;
mod exec;
mod faults;
mod forward;

use bed::Bed;

#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn exec_and_port_forward_on_k3s_carry_each_byte_and_tell_each_end() {
    let bed = Bed::start().await;
    exec::the_channels_carry_each_byte_and_the_status_gives_the_exit_code(&bed).await;
    exec::a_terminal_gets_its_size_and_a_missing_program_or_container_tells_why(&bed).await;
    forward::a_tunnel_reaches_a_web_server_in_a_pod_and_a_port_with_no_server_tells_why(&bed).await;
    // The faults come last: the silence breaks each connection of the cluster.
    faults::a_pod_that_reads_no_input_makes_the_input_wait_with_no_break(&bed).await;
    faults::a_silent_connection_and_a_reset_end_their_sockets_with_the_reason(&bed).await;
}
