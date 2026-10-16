//! The commands of the forwards view, and the saved rules of the settings file.

use std::net::{Ipv4Addr, TcpListener as StdListener};

use fresnel_core::app::Health;
use fresnel_core::config::forwards::{forget_edit, save_edits};
use fresnel_core::config::{FileRead, SaveState};
use fresnel_testkit::Fault;

use super::*;
use crate::app::CONFIG_FILE;

/// Plan section 15, M5: a local port in use gives the state "failed" with that reason. The
/// forward never takes an other port, and a start after the port is free binds it.
#[tokio::test(flavor = "multi_thread")]
async fn a_local_port_in_use_fails_with_that_reason_and_a_start_binds_it_once_free() {
    let bed = Bed::start("forward-in-use").await;
    let holder = StdListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a listener");
    let port = holder.local_addr().expect("its address").port();
    let fixed = ForwardRule {
        local: LocalPort::Fixed(port),
        bind: Bind::Address(Ipv4Addr::LOCALHOST.into()),
        ..rule(Kind::Pod, "web-1", 8080)
    };
    bed.start_forward(&fixed);
    let failed = bed.until(&fixed, "the state failed", |view| view.state.name() == "failed").await;
    let text = format!("failed: the local port {port} is in use: an other program listens on 127.0.0.1:{port}");
    assert_eq!((failed.view.state.to_string(), failed.view.addresses.len()), (text.clone(), 0));
    assert!(bed.app.journal().iter().any(|(_, source, entry)| source == "ams" && entry.ends_with(&text)));

    drop(holder);
    bed.app.core.send(Command::RestartForward(failed.id));
    let address = bed.active(&fixed).await;
    assert_eq!(address.port(), port);
    assert_eq!(echo(address, b"bytes").await, Ok(()));
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_start_again_retry_and_remove_act_on_their_forward_only() {
    let bed = Bed::start("forward-commands").await;
    let (one, two) = (rule(Kind::Pod, "web-1", 8080), rule(Kind::Pod, "web-1", 9090));
    bed.start_forward(&one);
    bed.start_forward(&two);
    let address = bed.active(&one).await;
    let other = bed.active(&two).await;
    let id = bed.until(&one, "its number", |_| true).await.id;
    let mut open = TcpStream::connect(address).await.expect("a connect");
    open.write_all(b"open").await.expect("a write");
    let mut back = [0_u8; 4];
    open.read_exact(&mut back).await.expect("the bytes back");

    bed.app.core.send(Command::StopForward(id));
    let stopped = bed.until(&one, "the state stopped", |view| view.state.name() == "stopped").await;
    assert_eq!(stopped.view.addresses, Vec::new());
    let end = timeout(10 * SECOND, open.read(&mut back)).await;
    assert!(matches!(end, Ok(Err(_))), "a stop resets each local connection: {end:?}");
    assert_eq!(echo(other, b"the other one runs").await, Ok(()));
    bed.start_forward(&one);
    let again = bed.active(&one).await;
    assert_eq!(echo(again, b"again").await, Ok(()));
    assert_eq!(bed.until(&one, "the same entry", |_| true).await.id, id, "an equal rule is the same forward");
    assert!(address.port() != 0);

    // "Retry now" tells the cluster to test its path now: its warm connections go.
    let dropped = bed.app.until("a warm connection", |view| upgrade(view).warm > 0).await;
    let dropped = upgrade(&dropped).dropped;
    bed.app.core.send(Command::RetryForward(id));
    bed.app.until_texts("ams", |texts| texts.iter().any(|text| text.ends_with(": retry now"))).await;
    bed.app.until("the test of the path", |view| upgrade(view).dropped > dropped).await;

    bed.app.core.send(Command::RemoveForward(id));
    let view = bed.app.until("the forward is gone", |view| forward(view, &one).is_none()).await;
    assert_eq!(view.forwards.len(), 1, "the other forward stays");
}

/// A saved rule is in the list from the start of the app and starts with it; a forget keeps
/// the forward that runs, and a save writes the rule back.
#[tokio::test(flavor = "multi_thread")]
async fn a_saved_rule_starts_with_the_app_and_a_forget_and_a_save_change_the_file_only() {
    let server = FakeApiServer::start().await.expect("a server");
    server.set_body(PODS, &list("PodList", &[pod("web-1", "u1")]));
    let home = home("forward-saved");
    write_kubeconfig(&home, "ams", &[("ams", server.addr(), &server, false)]);
    let saved = rule(Kind::Pod, "web-1", 8080);
    let edits = save_edits(&saved);
    let text = fresnel_core::config::edit::apply("version = 1\n", &edits).expect("a saved rule");
    home.write(CONFIG_FILE, &text).expect("the settings file");
    let app = App::started(&home, settings());
    app.until_health("ams", Health::Live).await;
    let active =
        |view: &CoreView| forward(view, &saved).is_some_and(|row| row.saved && row.view.state.name() == "active");
    let view = app.until("the saved forward runs", active).await;
    let id = forward(&view, &saved).expect("the row").id;

    let saved_result = |number| {
        move |view: &CoreView| {
            view.config_file.save == Some(SaveState { number, result: Some(Ok(())) })
                && matches!(view.config_file.read, FileRead::Read { used: true, .. })
        }
    };
    app.core.send(Command::EditSettings(vec![forget_edit(&saved)]));
    let view = app.until("the forget", saved_result(1)).await;
    let row = forward(&view, &saved).expect("the forward runs on");
    assert_eq!((row.id, row.saved, row.view.state.name()), (id, false, "active"));
    app.core.send(Command::EditSettings(edits));
    let view = app.until("the save", saved_result(2)).await;
    assert!(forward(&view, &saved).is_some_and(|row| row.saved));
}

/// A forward that the server refuses fails with the reason and keeps its fixed port. Its
/// "Start" after the user got the right closes the old listener before the new bind, so the
/// forward binds the same port again and carries the bytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_forward_fails_with_the_reason_and_a_start_binds_its_fixed_port_again() {
    let bed = Bed::start("forward-refused").await;
    bed.server.add_fault(Fault::forbidden().on_path("/api/v1/namespaces/shop/pods/web-1/portforward"));
    let port = StdListener::bind((Ipv4Addr::LOCALHOST, 0)).and_then(|free| free.local_addr()).expect("a port").port();
    let fixed = ForwardRule {
        local: LocalPort::Fixed(port),
        bind: Bind::Address(Ipv4Addr::LOCALHOST.into()),
        ..rule(Kind::Pod, "web-1", 8080)
    };
    bed.start_forward(&fixed);
    let address = bed.active(&fixed).await;
    assert_eq!(end_seen(address).await, Ok("reset"));
    let failed = bed.until(&fixed, "the state failed", |view| view.state.name() == "failed").await;
    let state = failed.view.state.to_string();
    assert!(state.starts_with("failed: the server refuses the port forward: "), "{state}");
    assert_eq!(failed.view.address(), Some(address), "the port stays bound");

    bed.server.clear_faults();
    bed.app.core.send(Command::RestartForward(failed.id));
    assert_eq!(bed.active(&fixed).await, address);
    assert_eq!(echo(address, b"bytes").await, Ok(()));
}

/// A local connection that waits for a tunnel tells the cluster to test its path now: a
/// break can be the cause, and the warm connections can be silent, so they go.
#[tokio::test(flavor = "multi_thread")]
async fn a_local_connection_that_waits_tells_the_cluster_to_test_its_path() {
    let bed = Bed::start("forward-waits").await;
    let gone = rule(Kind::Pod, "web-9", 8080);
    bed.start_forward(&gone);
    let degraded = bed.until(&gone, "no pod", |view| view.state.name() == "degraded").await;
    assert_eq!(degraded.view.state.to_string(), "degraded: the Pod shop/web-9 does not exist");
    let dropped = bed.app.until("a warm connection", |view| upgrade(view).warm > 0).await;
    let dropped = upgrade(&dropped).dropped;
    let _waits = TcpStream::connect(degraded.view.address().expect("an open port")).await.expect("a connect");
    bed.app.until("the test of the path", |view| upgrade(view).dropped > dropped).await;
}
