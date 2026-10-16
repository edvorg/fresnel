//! The commands of the forwards view, and the saved rules of the settings file.

use std::net::{Ipv4Addr, TcpListener as StdListener};

use fresnel_core::app::Health;
use fresnel_core::config::forwards::{forget_edit, save_edits};
use fresnel_core::config::{FileRead, SaveState};

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

    bed.app.core.send(Command::StopForward(id));
    let stopped = bed.until(&one, "the state stopped", |view| view.state.name() == "stopped").await;
    assert_eq!(stopped.view.addresses, Vec::new());
    assert_eq!(echo(other, b"the other one runs").await, Ok(()));
    bed.start_forward(&one);
    let again = bed.active(&one).await;
    assert_eq!(echo(again, b"again").await, Ok(()));
    assert_eq!(bed.until(&one, "the same entry", |_| true).await.id, id, "an equal rule is the same forward");
    assert!(address.port() != 0);

    bed.app.core.send(Command::RetryForward(id));
    bed.app.until_texts("ams", |texts| texts.iter().any(|text| text.ends_with(": retry now"))).await;

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
