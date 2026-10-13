//! A network that goes away and comes back.

use std::sync::atomic::Ordering;
use std::time::Duration;

use fresnel_core::app::{CoreSettings, Health};
use fresnel_core::journal::Level;
use fresnel_core::net::task::Backoff;
use fresnel_testkit::{FakeApiServer, FaultProxy, ProxyMode};

use super::{App, home, row, write_kubeconfig};
use crate::common::in_time;

/// The demonstration of milestone M0: the network goes away and comes back, and each state
/// change shows.
#[tokio::test(flavor = "multi_thread")]
async fn each_state_change_is_in_the_journal_and_a_network_change_ends_the_retry_delay() {
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    let home = home("network");
    write_kubeconfig(&home, "far", &[("far", proxy.addr(), &server, false)]);
    let mut settings = CoreSettings::default();
    settings.net.connect.connect_limit = Duration::from_millis(300);
    // With no signal, the next connect after a failure would start after 10 minutes.
    let never = Duration::from_secs(600);
    let backoff = Backoff { first: never, max: never, jitter_percent: 0, reset_after: Duration::ZERO };
    for channels in [&mut settings.net.watches, &mut settings.net.requests, &mut settings.net.logs] {
        channels.backoff = backoff;
    }
    let app = App::started(&home, settings);
    app.until_health("far", Health::Live).await;

    // The network is gone: each connection ends, and a new one gets no answer.
    proxy.set(ProxyMode::Freeze);
    in_time("the reset of each connection", proxy.reset_all()).await;
    let lost = app.until_health("far", Health::NoConnection).await;
    let reason = row(&lost, "far").reason().expect("a reason");
    assert!(reason.contains("no answer in 300ms") || reason.contains("connect"), "{reason}");

    // The network is back. The detector tells the core, and the channels connect at once.
    proxy.set(ProxyMode::Pass);
    app.network_changed.store(true, Ordering::Relaxed);
    let back = tokio::time::Instant::now();
    app.until_health("far", Health::Live).await;
    assert!(back.elapsed() < Duration::from_secs(10), "{:?}", back.elapsed());

    let steps: Vec<String> = app
        .texts_of("far")
        .into_iter()
        .filter_map(|text| text.split([':', '(']).next().map(|step| step.trim().to_owned()))
        .filter(|step| step != "a connection was lost")
        .collect();
    assert_eq!(steps, ["the channels connect", "live", "no connection", "live"]);
    let network = "the network changed: each cluster tests its connections and ends its retry delays";
    assert_eq!(app.texts_of("app").last().map(String::as_str), Some(network));
    let levels: Vec<Level> = app.journal().into_iter().map(|(level, _, _)| level).collect();
    assert!(levels.contains(&Level::Warning) && !levels.contains(&Level::Error), "{:?}", app.journal());
}
