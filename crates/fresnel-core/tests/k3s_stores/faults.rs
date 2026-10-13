//! Resets at the worst moments, a restart of the API server and a silent connection.
//!
//! The steps are in one test, because the start of a cluster is the slow part.

use std::time::{Duration, Instant, SystemTime};

use fresnel_core::kinds::Kind;
use fresnel_core::store::{ObjectKey, StoreSettings, StoreState, StoreStatus, WatchSettings};
use fresnel_testkit::ProxyMode;
use tokio::time::sleep;

use crate::api::{self, ConfigMaps, assert_equal_to_the_api};
use crate::bed::{App, Cluster, LIMIT, RECOVERY, SHOWS_IN, none_live};
use crate::common::{in_time, until_live};

/// The ConfigMaps of the test. With pages of 25, their list has 13 pages.
const SEEDED: usize = 300;
/// One way through the proxy. A list page then needs 20 ms at least, so a reset can
/// come in the middle of a list.
const LATENCY: Duration = Duration::from_millis(10);
const SECRET_VALUE: &str = "ZnJlc25lbC10ZXN0LXZhbHVl";

fn settings() -> StoreSettings {
    StoreSettings { watch: WatchSettings { page_size: 25, ..WatchSettings::default() }, ..StoreSettings::default() }
}

fn config_maps(stores: &[StoreStatus]) -> &StoreStatus {
    stores.iter().find(|store| store.kind == Kind::ConfigMap).expect("the store of the ConfigMaps")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn the_stores_are_equal_to_the_api_after_each_fault() {
    let cluster = Cluster::start("stores-faults").await;
    let api = &cluster.api.gateway;
    ConfigMaps::of(api).create_many("seeded", SEEDED).await;
    api::create_applied_secret(api, "applied", SECRET_VALUE).await;
    let mut app = App::start(&cluster, settings()).await;
    app.proxy.set(ProxyMode::Latency(LATENCY));

    the_watches_of_each_kind_use_three_connections(&cluster, &app).await;
    resets_in_the_middle_of_a_list(&cluster, &mut app).await;
    resets_during_changes(&cluster, &app).await;
    a_restart_of_the_api_server(&cluster, &app).await;
    a_silent_connection(&cluster, &app).await;
    assert_eq!(app.proxy.task_failures(), Vec::<String>::new());
}

/// Plan section 15: 30 watches use 3 connections or fewer.
async fn the_watches_of_each_kind_use_three_connections(cluster: &Cluster, app: &App) {
    let started = Instant::now();
    let stores = app.stores.until_live(LIMIT).await;
    println!("{} stores are live {:?} after their start", stores.len(), started.elapsed());
    // The channel for log streams carries no request of a store, so it can connect later. A
    // connect attempt that lost its race closes at once. Then three connections stay.
    until_live(&app.link).await;
    let counters = in_time("three open connections", app.proxy.wait_for(|counters| counters.open <= 3)).await;
    let status = app.link.net.status();
    let streams: usize = status.watches.channels.iter().map(|channel| channel.streams_open).sum();
    println!("{streams} watches on {} connections; the proxy accepted {}", counters.open, counters.accepted);
    assert_eq!((stores.len(), streams, status.watches.channels.len()), (39, 39, 1), "each watch is on one connection");
    assert_eq!(counters.open, 3, "one connection for watches, one for lists and one for log streams");
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;

    // The server sends the copy of `kubectl apply` also with the metadata form. No store keeps it.
    let secrets = app.stores.stores.metadata(Kind::Secret).expect("the store of the secrets").read();
    let stored = secrets.items.get(&ObjectKey::new("default", "applied")).expect("the secret of the test");
    assert_eq!((&stored.metadata.annotations, &stored.metadata.managed_fields), (&None, &None));
    assert!(!format!("{stored:?}").contains(SECRET_VALUE), "a store holds a value of a Secret");
}

/// Two resets of each connection, each one in the middle of the list of the ConfigMaps.
async fn resets_in_the_middle_of_a_list(cluster: &Cluster, app: &mut App) {
    app.start_stores_again(cluster, settings());
    for reset in 1..=2 {
        let in_the_middle = |stores: &[StoreStatus]| matches!(config_maps(stores).state, StoreState::Listing { loaded } if (50..SEEDED).contains(&loaded));
        let listing = app.stores.until("the list of the ConfigMaps is in progress", LIMIT, in_the_middle).await;
        in_time("the reset", app.proxy.reset_all()).await;
        let failed = |stores: &[StoreStatus]| !matches!(config_maps(stores).state, StoreState::Listing { .. });
        let after = app.stores.until("the list failed", LIMIT, failed).await;
        let (before, after) = (config_maps(&listing), config_maps(&after));
        println!("reset {reset} came at `{}`; then the store showed `{}`", before.state, after.state);
        assert_eq!(after.items, 0, "half of a list is not a list");
        assert!(!after.state.is_live(), "{}", after.state);
    }
    let reset = Instant::now();
    app.stores.until_live(RECOVERY).await;
    println!("each store was live {:?} after the second reset", reset.elapsed());
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;
}

/// Creates, changes and deletes of ConfigMaps for some seconds, with a reset of each
/// connection of the app each 700 ms.
async fn resets_during_changes(cluster: &Cluster, app: &App) {
    let config_maps = ConfigMaps::of(&cluster.api.gateway);
    let changes = async {
        for number in 0..60 {
            config_maps.create(&format!("made-{number:03}")).await;
            config_maps.label(&format!("seeded-{number:03}"), "changed").await;
            config_maps.delete(&format!("seeded-{:03}", 100 + number)).await;
            sleep(Duration::from_millis(40)).await;
        }
    };
    let resets = async {
        let mut last = SystemTime::now();
        for _ in 0..5 {
            sleep(Duration::from_millis(700)).await;
            last = SystemTime::now();
            in_time("the reset", app.proxy.reset_all()).await;
        }
        last
    };
    let ((), last_reset) = tokio::join!(changes, resets);
    let recovery = app.live_again_after(last_reset, RECOVERY).await;
    println!("each store was live {recovery:?} after the last of 5 resets during 180 changes");
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;
    assert_eq!(
        api::stored(&app.stores.stores, Kind::ConfigMap).keys().filter(|(_, name)| name.starts_with("made-")).count(),
        60
    );
}

/// The API server stops and starts again. Its watch cache is new then, so a watch from a
/// version before the restart can get HTTP 410.
async fn a_restart_of_the_api_server(cluster: &Cluster, app: &App) {
    let config_maps = ConfigMaps::of(&cluster.api.gateway);
    config_maps.create("before-the-restart").await;
    let stopped = (Instant::now(), SystemTime::now());
    cluster.k3s.restart().await.expect("k3s starts again");
    let restart = stopped.0.elapsed();
    let live = app.live_again_after(stopped.1, RECOVERY).await;
    println!("the server was ready {restart:?} after its stop; each store was live again {live:?} after the stop");
    assert!(live < restart + RECOVERY, "the stores need more than 10 s after the server is ready");
    until_live(&cluster.api).await;
    config_maps.create("after-the-restart").await;
    config_maps.delete("before-the-restart").await;
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;
}

/// Each open connection carries no data and gives no error, as after a wake of the
/// laptop. New connections work.
async fn a_silent_connection(cluster: &Cluster, app: &App) {
    app.proxy.silence_open();
    let silent = Instant::now();
    ConfigMaps::of(&cluster.api.gateway).create("during-the-silence").await;
    app.stores.until("no store is live", SHOWS_IN, none_live).await;
    let shown = (silent.elapsed(), SystemTime::now());
    let recovery = app.live_again_after(shown.1, RECOVERY).await;
    println!("the silent connection showed in each store after {:?}; each store was live {recovery:?} later", shown.0);
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;
}
