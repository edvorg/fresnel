//! A network with no answer for 30 seconds, 2 minutes and 10 minutes (plan section 15).
//!
//! The three breaks run at the same time, each one for an app of its own before the same
//! cluster. Thus the test needs the time of the longest break.
//!
//! The proxy counts each connect attempt of the app in a break. M1 measured 2,917 in the
//! break of 10 minutes, with no limit. Now the channels of a cluster take turns after the
//! first minute of an outage, with one attempt for each turn of 4 to 6 s.

use std::time::{Duration, SystemTime};

use fresnel_core::store::StoreSettings;
use fresnel_testkit::ProxyMode;
use tokio::time::{Instant, sleep_until};

use crate::api::{ConfigMaps, assert_equal_to_the_api};
use crate::bed::{App, Cluster, LIMIT, RECOVERY, SHOWS_IN, none_live};

/// The channels take turns from this time of a break at the latest: one minute after the
/// first connect that failed, which starts when the silent connections show, plus the time
/// limit of the connects in progress.
const PACED_FROM: Duration = Duration::from_secs(120);
/// The shortest and the longest time from one turn to the next.
const TURNS: (u64, u64) = (4, 6);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn each_store_is_live_10_seconds_after_the_end_of_a_break_of_30_s_of_2_minutes_and_of_10_minutes() {
    let cluster = Cluster::start("stores-breaks").await;
    ConfigMaps::of(&cluster.api.gateway).create_many("seeded", 30).await;
    let mut apps = Vec::new();
    for seconds in [30, 120, 600] {
        apps.push((App::start(&cluster, StoreSettings::default()).await, Duration::from_secs(seconds)));
    }
    let breaks = apps.iter().enumerate().map(|(number, (app, length))| one_break(&cluster, app, number, *length));
    futures_util::future::join_all(breaks).await;
    for (app, _) in &apps {
        assert_eq!(app.proxy.task_failures(), Vec::<String>::new());
    }
}

/// Stops each byte between the app and the server for `length`. The cluster changes in
/// that time: one new ConfigMap, and the ConfigMap `number` of the test goes. Then the
/// network returns.
async fn one_break(cluster: &Cluster, app: &App, number: usize, length: Duration) {
    let seconds = length.as_secs();
    let config_maps = ConfigMaps::of(&cluster.api.gateway);
    app.stores.until_live(LIMIT).await;
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;

    app.proxy.set(ProxyMode::Freeze);
    let cut = Instant::now();
    config_maps.create(&format!("made-in-the-break-of-{seconds}-s")).await;
    app.stores.until("no store is live", SHOWS_IN, none_live).await;
    println!("break of {seconds} s: each store showed it after {:?}", cut.elapsed());

    // A turn costs one attempt. The count needs a part of a break with turns only.
    let paced_part = length.checked_sub(PACED_FROM).filter(|part| *part >= Duration::from_secs(60));
    let mut at_pace = None;
    if paced_part.is_some() {
        sleep_until(cut + PACED_FROM).await;
        at_pace = Some(app.proxy.counters().gave_up);
    }

    // The app must keep what it has, with a state that tells its age.
    sleep_until(cut + length - Duration::from_secs(1)).await;
    let during = app.stores.stores.statuses();
    assert!(none_live(&during), "a store is live during the break: {:#?}", app.stores.states());
    assert!(during.iter().map(|store| store.items).sum::<usize>() > 30, "the stores keep their objects");
    config_maps.delete(&format!("seeded-{number:03}")).await;
    sleep_until(cut + length).await;
    let attempts = app.proxy.counters().gave_up;

    let back = SystemTime::now();
    app.proxy.set(ProxyMode::Pass);
    let recovery = app.live_again_after(back, RECOVERY).await;
    println!("break of {seconds} s: each store was live {recovery:?} after the network returned");
    println!("break of {seconds} s: {attempts} connect attempts of the app gave up and never reached the server");
    if let Some((part, from)) = paced_part.zip(at_pace) {
        let (paced, part) = (attempts - from, part.as_secs());
        println!("break of {seconds} s: {paced} connect attempts in its last {part} s, in turns");
        let (fewest, most) = (part / TURNS.1 - 1, part / TURNS.0 + 1);
        assert!((fewest..=most).contains(&paced), "one attempt for each turn: {paced} in {part} s");
    }
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;
}
