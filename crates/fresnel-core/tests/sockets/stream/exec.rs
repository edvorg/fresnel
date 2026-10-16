//! The channels of exec and attach: the input, the output, the size, the exit and the journal.

use bytes::Bytes;
use fresnel_core::net::ErrorClass;
use fresnel_core::stream::{ExecEnd, Exit};
use fresnel_testkit::Fault;

use super::*;

#[tokio::test]
async fn an_exec_sends_its_input_back_and_ends_with_the_exit_code_of_its_status() {
    let bed = Bed::start(NetSettings::default()).await;
    let journal = journal();
    let mut session = ExecSession::open(&bed.link.gateway, &sh(), journal.clone()).await.expect("an exec");
    let input = session.input();
    input.send(b"echo one\n").await.expect("the input");
    assert_eq!(in_time("the echo", session.next()).await, Some(Output::Stdout(Bytes::from_static(b"echo one\n"))));
    input.resize(132, 43).await.expect("a size");
    let size = Output::Stderr(Bytes::from_static(b"size {\"Width\":132,\"Height\":43}\n"));
    assert_eq!(in_time("the size", session.next()).await, Some(size));
    assert_eq!(session.end(), None, "the process runs");

    input.send(b"exit 3\n").await.expect("the input");
    assert_eq!(output(&mut session).await, (b"exit 3\n".to_vec(), Vec::new()));
    assert_eq!(session.end(), Some(ExecEnd::Exited(Exit::Code(3))));
    assert_eq!(
        entries(&journal),
        [
            "Info bed: an exec in pod default/web: open, v5.channel.k8s.io",
            "Info bed: an exec in pod default/web: the process ended with the exit code 3",
        ]
    );
    let sockets = closed_sockets(&bed).await;
    assert_eq!((sockets.opened, sockets.closed, sockets.failed), (1, 1, 0));
    // Two messages of input and one of the size, each with its channel byte.
    assert_eq!(sockets.sent, (9 + 1) + (25 + 1) + (7 + 1));
}

/// v5 ends the input as a pipe does. A server before Kubernetes 1.30 selects v4, which has
/// no such signal.
#[tokio::test]
async fn the_end_of_the_input_of_v5_ends_the_process_and_v4_refuses_it() {
    let bed = Bed::start(NetSettings::default()).await;
    let mut session = ExecSession::open(&bed.link.gateway, &sh(), journal()).await.expect("an exec");
    session.input().close_stdin().await.expect("the end of the input");
    assert_eq!(output(&mut session).await, (Vec::new(), Vec::new()));
    assert_eq!(session.end(), Some(ExecEnd::Exited(Exit::Code(0))));

    bed.server.exec_with_v4_only();
    let journal = journal();
    let attach = Exec { container: Some("app".to_owned()), command: None, tty: true, ..sh() };
    let mut session = ExecSession::open(&bed.link.gateway, &attach, journal.clone()).await.expect("an attach");
    let error = session.input().close_stdin().await.expect_err("v4 cannot end the input");
    assert_eq!(error.to_string(), "the server has the subprotocol v4, which cannot end the input");
    session.input().send(b"exit 0\n").await.expect("the input");
    assert_eq!(output(&mut session).await, (b"exit 0\n".to_vec(), Vec::new()));
    assert_eq!(session.end(), Some(ExecEnd::Exited(Exit::Code(0))));
    assert_eq!(entries(&journal)[0], "Info bed: an attach to pod default/web (container app): open, v4.channel.k8s.io");
    let requests = bed.server.requests();
    let attached =
        requests.iter().filter_map(|seen| seen.uri.path_and_query()).find(|uri| uri.path().ends_with("/attach"));
    let attached = attached.map(|uri| uri.as_str());
    assert_eq!(
        attached,
        Some("/api/v1/namespaces/default/pods/web/attach?&stdin=true&stdout=true&tty=true&container=app")
    );
}

#[tokio::test]
async fn a_refused_exec_tells_the_message_of_the_server_in_its_error_and_the_journal() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.add_fault(Fault::forbidden().on_path("/api/v1/namespaces/default/pods/web/exec"));
    let journal = journal();
    let error = ExecSession::open(&bed.link.gateway, &sh(), journal.clone()).await.expect_err("refused");
    let text = "forbidden: the user of the test cannot read this path (HTTP 403)";
    assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Forbidden, text));
    assert_eq!(entries(&journal), [format!("Warning bed: an exec in pod default/web: no start: {text}")]);
}
