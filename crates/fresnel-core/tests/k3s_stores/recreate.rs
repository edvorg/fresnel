//! A new cluster at the same address: the recreate of the soak, with a new datastore. Each
//! object of the old cluster is gone, a new object of the same name has an other UID, and
//! the resource versions start again.
//!
//! A store that holds an object of the old cluster is behind the API. A 24-hour soak saw the
//! store of the ConfigMaps "live" with such objects for 20 s after a recreate (design rule 4).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use fresnel_core::kinds::{Kind, Tier};
use fresnel_core::store::{ClusterStores, StoreSettings, StoreStatus};
use fresnel_testkit::ProxyMode;
use tokio::sync::watch;
use tokio::time::{Instant, sleep};

use crate::api::{ConfigMaps, assert_equal_to_the_api, look};
use crate::bed::{App, Cluster, LIMIT, SHOWS_IN, none_live};
use crate::common::until_live;

/// The longest time between two looks at the stores: a look also follows each new snapshot.
const LOOK: Duration = Duration::from_millis(20);
/// More than the versions that the old cluster gives after the test read its version: its
/// leases change each few seconds.
const MARGIN: u64 = 500;

/// When the app reaches the new cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// As soon as its API server answers. Its resource version is often older than the
    /// cursors of the stores then, and the check of the version gives a list.
    AtOnce,
    /// After its resource version passed each cursor of the stores. The check of the version
    /// passes then, and only the identity of the cluster tells the change.
    PastTheVersion,
}

/// The kinds of the stores that are always on.
fn kinds() -> Vec<Kind> {
    Kind::ALL.iter().copied().filter(|kind| kind.info().tier != Tier::C).collect()
}

/// The UIDs of the objects of each store now.
fn uids(stores: &ClusterStores) -> BTreeMap<Kind, BTreeSet<String>> {
    let of = |kind: Kind| look(stores, kind).objects.into_values().map(|(uid, _)| uid).collect();
    kinds().into_iter().map(|kind| (kind, of(kind))).collect()
}

/// What the looks saw of one store after the end of the old cluster.
#[derive(Default)]
struct Seen {
    /// The looks that saw the store "live" with an object of the old cluster.
    behind: usize,
    /// Since when the store shows "live" with such an object.
    since: Option<Instant>,
    /// The longest time from the first look of that kind to the first one after it.
    longest: Duration,
    /// Each state in words, in the sequence of the looks, with no repeat.
    states: Vec<String>,
}

impl Seen {
    fn see(&mut self, live_and_behind: bool, words: String, now: Instant) {
        if live_and_behind {
            self.behind += 1;
            self.since.get_or_insert(now);
        } else if let Some(since) = self.since.take() {
            self.longest = self.longest.max(now - since);
        }
        if self.states.last() != Some(&words) {
            self.states.push(words);
        }
    }
}

/// Looks at each store at each new snapshot and each `LOOK` until `done` is true. A look
/// reads the state and the objects of one snapshot, so a store that lists the new cluster
/// before it shows "live" never shows both.
async fn live_and_behind(
    stores: &ClusterStores,
    old: &BTreeMap<Kind, BTreeSet<String>>,
    mut done: watch::Receiver<bool>,
) -> BTreeMap<Kind, Seen> {
    let mut seen: BTreeMap<Kind, Seen> = kinds().into_iter().map(|kind| (kind, Seen::default())).collect();
    let mut changes = stores.changes();
    loop {
        let (now, last) = (Instant::now(), *done.borrow());
        for (kind, seen) in &mut seen {
            let look = look(stores, *kind);
            let old = old.get(kind);
            let behind = old.is_some_and(|old| look.objects.values().any(|(uid, _)| old.contains(uid)));
            seen.see(look.state.is_live() && behind, look.state.to_string(), now);
        }
        if last {
            return seen;
        }
        tokio::select! {
            () = changes.next() => {}
            () = sleep(LOOK) => {}
            _ = done.changed() => {}
        }
    }
}

/// The stores that are not live or that hold an object of the old cluster, with the count
/// of such objects. Empty when each store caught up with the new cluster.
fn behind(stores: &ClusterStores, statuses: &[StoreStatus], old: &BTreeMap<Kind, BTreeSet<String>>) -> Vec<String> {
    let current = uids(stores);
    let old_objects =
        |kind: &Kind| old.get(kind).zip(current.get(kind)).map_or(0, |(old, now)| old.intersection(now).count());
    let line =
        |store: &StoreStatus| format!("{:?}: {}, {} old objects", store.kind, store.state, old_objects(&store.kind));
    statuses.iter().filter(|store| !store.state.is_live() || old_objects(&store.kind) > 0).map(line).collect()
}

/// Waits until each store is live and holds no object of the old cluster. Gives the stores
/// that did not catch up in `LIMIT`.
async fn until_caught_up(stores: &ClusterStores, old: &BTreeMap<Kind, BTreeSet<String>>) -> Vec<String> {
    let mut changes = stores.changes();
    let end = Instant::now() + LIMIT;
    loop {
        let left = behind(stores, &stores.statuses(), old);
        if left.is_empty() || Instant::now() >= end {
            return left;
        }
        tokio::select! {
            () = changes.next() => {}
            () = sleep(LOOK) => {}
        }
    }
}

/// Two recreates in a row: the first as the keeper of the soak makes it, the second with a
/// new cluster that passed the version of each store before the app reaches it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn no_store_shows_live_with_the_objects_of_the_old_cluster_after_a_recreate() {
    let cluster = Cluster::start("stores-recreate").await;
    ConfigMaps::of(&cluster.api.gateway).create_many("seeded", 30).await;
    let app = App::start(&cluster, StoreSettings::default()).await;
    app.stores.until_live(LIMIT).await;
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;
    recreate(&cluster, &app, Reach::AtOnce).await;
    recreate(&cluster, &app, Reach::PastTheVersion).await;
    assert_eq!(app.proxy.task_failures(), Vec::<String>::new());
}

/// One recreate behind the stores. The looks start when no store is live, and no store may
/// show "live" with an object of the old cluster.
async fn recreate(cluster: &Cluster, app: &App, reach: Reach) {
    let config_maps = ConfigMaps::of(&cluster.api.gateway);
    let old = uids(&app.stores.stores);
    let old_version = config_maps.version_now().await;
    let (done, until_done) = watch::channel(false);
    let (looking, mut until_looking) = watch::channel(false);
    let started = Instant::now();
    let recreate = async {
        if reach == Reach::PastTheVersion {
            app.proxy.set(ProxyMode::Freeze);
        }
        cluster.k3s.recreate().await.expect("a new cluster");
        println!("{reach:?}: the new cluster was ready {:?} after the start of the recreate", started.elapsed());
        until_live(&cluster.api).await;
        // The objects of the soak again: the same names, with other UIDs.
        config_maps.create_many("seeded", 30).await;
        if reach == Reach::PastTheVersion {
            pass(&config_maps, old_version + MARGIN).await;
            until_looking.wait_for(|looking| *looking).await.expect("the looks");
            app.proxy.set(ProxyMode::Pass);
        }
        let left = until_caught_up(&app.stores.stores, &old).await;
        println!("{reach:?}: each store was live with the new objects {:?} after the start", started.elapsed());
        done.send_replace(true);
        left
    };
    // The stores are current until the old cluster stops. The looks start when no store is live.
    let looks = async {
        app.stores.until("no store is live", SHOWS_IN, none_live).await;
        println!("{reach:?}: no store was live {:?} after the start of the recreate", started.elapsed());
        looking.send_replace(true);
        live_and_behind(&app.stores.stores, &old, until_done).await
    };
    let (left, seen) = tokio::join!(recreate, looks);

    for (kind, seen) in &seen {
        let (name, states) = (kind.info().name, seen.states.join(", "));
        println!(
            "{reach:?}: {name}: {} looks, {:?} live with an old object; states: {states}",
            seen.behind, seen.longest
        );
    }
    let shown: Vec<String> = seen
        .iter()
        .filter(|(_, seen)| seen.behind > 0)
        .map(|(kind, seen)| format!("{}: {} looks, {:?}", kind.info().name, seen.behind, seen.longest))
        .collect();
    assert_eq!(left, Vec::<String>::new(), "{reach:?}: stores that did not catch up with the new cluster in {LIMIT:?}");
    assert_eq!(shown, Vec::<String>::new(), "{reach:?}: stores that showed \"live\" with an object of the old cluster");
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;
}

/// Changes the ConfigMaps of the test until the resource version of the cluster passes
/// `version`. Each write takes the next version.
async fn pass(config_maps: &ConfigMaps<'_>, version: u64) {
    let names: Vec<String> = (0..30).map(|number| format!("seeded-{number:03}")).collect();
    let mut round = 0_u32;
    while config_maps.version_now().await <= version {
        round += 1;
        let value = round.to_string();
        futures_util::future::join_all(names.iter().map(|name| config_maps.label(name, &value))).await;
    }
    println!("the version of the new cluster passed {version} after {round} rounds of 30 writes");
}
