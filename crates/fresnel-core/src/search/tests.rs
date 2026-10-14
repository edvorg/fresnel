//! The quick search on the objects of the two scenes of k3s, as the stores of the app hold
//! them: the full objects of tier A, and the metadata of tier B.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use k8s_openapi::api::core::v1::{Pod, Secret, Service};
use k8s_openapi::api::networking::v1::Ingress;
use tokio::runtime::{Builder, Runtime};

use super::*;
use crate::error::CoreError;
use crate::fixture;
use crate::kinds::{Describe, Meta, Object};
use crate::net::task::Tasks;
use crate::published::{Disposal, Held, Notice, Publisher};
use crate::store::Snapshot;

/// The time of the last data of a store of the tests.
fn contact() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

/// Gives the snapshots of the tests to a reader, as a store does.
struct Bed {
    disposal: Disposal,
    _runtime: Runtime,
}

impl Bed {
    fn new() -> Self {
        let runtime = Builder::new_current_thread().build().expect("a runtime");
        let tasks = Tasks::new(runtime.handle().clone(), |report| panic!("a task ended: {report}"));
        Self { disposal: Disposal::start(&tasks), _runtime: runtime }
    }

    fn held<T: Send + Sync + 'static>(&self, value: T) -> Held<T> {
        Publisher::new(value, self.disposal.bin(), Notice::new()).read()
    }

    fn full<K: Describe>(&self, state: StoreState, objects: Vec<K>) -> Source {
        Source::full(self.held(snapshot(state, objects)))
    }

    /// The store of a kind of tier B: the metadata of the objects only.
    fn metadata<K: Describe>(&self, state: StoreState, objects: Vec<K>) -> Source {
        let metas = objects.into_iter().map(|object| Meta { metadata: object.meta().clone() }).collect();
        Source::metadata(K::ID, self.held(snapshot(state, metas)))
    }
}

fn snapshot<T: Object>(state: StoreState, objects: Vec<T>) -> Snapshot<T> {
    Snapshot {
        generation: 1,
        state,
        since: contact(),
        last_contact: Some(contact()),
        listed: true,
        dropped: 0,
        items: objects.into_iter().map(Arc::new).collect(),
    }
}

fn secret() -> Secret {
    fixture::of_kinds("web-secret", |_| {})
}

fn forbidden() -> StoreState {
    StoreState::Forbidden { reason: CoreError::answered(403, "secrets is forbidden (HTTP 403)") }
}

/// A cluster with the pods and the Services of the scene, and its Secrets in this state.
fn cluster(bed: &Bed, name: &str, secrets: StoreState) -> ClusterSources {
    let objects = if secrets.is_live() { vec![secret()] } else { Vec::new() };
    let sources = vec![
        bed.full(StoreState::Live, fixture::all::<Pod>()),
        bed.full(StoreState::Live, fixture::all::<Service>()),
        bed.metadata(secrets, objects),
    ];
    ClusterSources { cluster: ClusterId::new(name), sources: Ok(sources) }
}

fn search(text: &str, clusters: &[ClusterSources]) -> Found {
    let query = Query { text: text.to_owned(), ..Query::default() };
    run(&query, clusters, &|| false).expect("no cancel")
}

/// The hits as `cluster kind namespace/name`.
fn lines(found: &Found) -> Vec<String> {
    let line = |hit: &Hit| format!("{} {:?} {}/{}", hit.cluster.as_str(), hit.kind, hit.key.namespace, hit.key.name);
    found.hits.iter().map(line).collect()
}

/// Weakness 5 of plan section 6: a kind that the search does not cover. The store of the
/// Secrets has their metadata only, and the name of a Secret gives its hit.
#[test]
fn w5_the_name_of_a_secret_gives_a_hit_with_its_cluster_and_its_kind() {
    let bed = Bed::new();
    let found = search("web-secret", &[cluster(&bed, "ams-prod", StoreState::Live)]);
    assert_eq!(lines(&found), ["ams-prod Secret kinds/web-secret"]);
    assert_eq!(found.hits[0].stale, None, "a live store");
    let fuzzy = search("wbscrt", &[cluster(&bed, "ams-prod", StoreState::Live)]);
    assert_eq!(lines(&fuzzy), ["ams-prod Secret kinds/web-secret"], "the letters of the name in their sequence");
}

/// Weakness 6 of plan section 6: a search failure with no message. The user cannot list the
/// Secrets of one cluster: the result names that store with the reason, and has the hits
/// of the other cluster.
#[test]
fn w6_a_forbidden_store_gives_a_note_with_its_reason_and_the_other_hits_stay() {
    let bed = Bed::new();
    let clusters = [cluster(&bed, "ams-prod", StoreState::Live), cluster(&bed, "fra-stage", forbidden())];
    let found = search("web-secret", &clusters);
    assert_eq!(lines(&found), ["ams-prod Secret kinds/web-secret"]);
    let refused: Vec<&Unread> = found.unread.iter().filter(|unread| matches!(unread.why, Why::Refused(_))).collect();
    let [note] = refused.as_slice() else { panic!("one note of a refusal: {:#?}", found.unread) };
    assert_eq!((note.cluster.as_str(), note.kind), ("fra-stage", Some(Kind::Secret)));
    assert_eq!(note.why.to_string(), "forbidden: secrets is forbidden (HTTP 403)");

    // The same note with a text that matches nothing: no hit is no proof of no object.
    let none = search("no-such-object", &clusters);
    assert_eq!((none.hits.len(), none.total), (0, 0));
    assert!(
        none.unread.iter().any(|unread| unread.cluster.as_str() == "fra-stage" && unread.kind == Some(Kind::Secret))
    );
}

#[test]
fn a_hit_of_a_store_that_is_not_live_has_the_time_of_its_last_data() {
    let bed = Bed::new();
    let backoff = StoreState::Backoff { failures: 2, reason: CoreError::invalid("reset"), until: contact() };
    let sources = vec![bed.full(backoff, fixture::all::<Pod>())];
    let found = search("crash", &[ClusterSources { cluster: ClusterId::new("ams-prod"), sources: Ok(sources) }]);
    assert!(!found.hits.is_empty());
    assert!(found.hits.iter().all(|hit| hit.stale == Some(Stale { last_contact: Some(contact()) })));
}

#[test]
fn each_store_that_the_search_did_not_read_in_full_is_a_note_with_its_reason() {
    let bed = Bed::new();
    let listing = StoreState::Listing { loaded: 3 };
    let partial = Source::full(bed.held(Snapshot { listed: false, ..snapshot(listing, fixture::all::<Pod>()) }));
    let clusters = [
        ClusterSources { cluster: ClusterId::new("ams-prod"), sources: Ok(vec![partial]) },
        ClusterSources {
            cluster: ClusterId::new("nyc-prod"),
            sources: Err("the connect did not start: no server".into()),
        },
    ];
    let found = search("crash", &clusters);
    assert!(!found.hits.is_empty(), "the objects of a list in progress");
    let note = |cluster: &str, kind| {
        found.unread.iter().find(|unread| unread.cluster.as_str() == cluster && unread.kind == kind)
    };
    let incomplete = note("ams-prod", Some(Kind::Pod)).map(|unread| unread.why.to_string());
    assert_eq!(
        incomplete.as_deref(),
        Some("its first list is not complete (list in progress, 3 objects); the search read a part")
    );
    let no_stores = note("nyc-prod", None).map(|unread| unread.why.to_string());
    assert_eq!(no_stores.as_deref(), Some("no store: the connect did not start: no server"));
    // The kinds of tier C have a store only while a page shows them.
    assert_eq!(note("ams-prod", Some(Kind::Lease)).map(|unread| &unread.why), Some(&Why::NoStore));
    assert_eq!(note("ams-prod", Some(Kind::Endpoints)).map(|unread| &unread.why), Some(&Why::NoStore));
}

#[test]
fn the_cluster_and_the_kind_are_filters_and_a_filter_limits_the_notes_too() {
    let bed = Bed::new();
    let clusters = [cluster(&bed, "ams-prod", StoreState::Live), cluster(&bed, "fra-stage", forbidden())];
    let query = |clusters_: &[&str], kinds: &[Kind]| Query {
        text: "web".to_owned(),
        clusters: clusters_.iter().map(|name| ClusterId::new(*name)).collect(),
        kinds: kinds.iter().copied().collect(),
    };
    let found = run(&query(&["fra-stage"], &[Kind::Service]), &clusters, &|| false).expect("no cancel");
    assert!(found.hits.iter().all(|hit| hit.cluster.as_str() == "fra-stage" && hit.kind == Kind::Service));
    assert!(!found.hits.is_empty());
    assert_eq!(found.unread, [], "the forbidden Secrets are outside the filter");

    let found = run(&query(&[], &[Kind::Secret]), &clusters, &|| false).expect("no cancel");
    assert_eq!(lines(&found), ["ams-prod Secret kinds/web-secret"]);
    assert_eq!(found.unread.len(), 1);
}

#[test]
fn the_best_hits_come_first_and_the_total_counts_each_one() {
    let bed = Bed::new();
    let pods: Vec<Pod> = (0..500)
        .map(|number| {
            let mut pod = fixture::one::<Pod>("crash");
            pod.metadata.name = Some(format!("crash-copy-{number:03}"));
            pod
        })
        .chain([fixture::one::<Pod>("crash")])
        .collect();
    let sources = vec![bed.full(StoreState::Live, pods)];
    let found = search("crash", &[ClusterSources { cluster: ClusterId::new("ams-prod"), sources: Ok(sources) }]);
    assert_eq!((found.total, found.hits.len()), (501, MAX_HITS));
    assert_eq!(found.hits[0].key.name.as_ref(), "crash", "the name itself first");
    assert_eq!(found.hits[1].key.name.as_ref(), "crash-copy-000", "then by name");
}

#[test]
fn the_full_objects_of_a_kind_that_a_page_shows_give_the_texts_of_the_kind() {
    let bed = Bed::new();
    let ingress = || fixture::of_kinds::<Ingress>("web", |_| {});
    let metadata = bed.metadata(StoreState::Live, vec![ingress()]);
    let full = bed.full(StoreState::Live, vec![ingress()]);
    let host = |source: Source| {
        search("web.example.com", &[ClusterSources { cluster: ClusterId::new("a"), sources: Ok(vec![source]) }])
    };
    assert_eq!(host(metadata).total, 0, "the metadata has no host");
    assert_eq!(lines(&host(full)), ["a Ingress kinds/web"], "the host of a rule");
}

#[test]
fn a_cancel_stops_the_search_and_an_empty_text_reads_no_object() {
    let bed = Bed::new();
    let clusters = [cluster(&bed, "ams-prod", StoreState::Live)];
    let asked = AtomicUsize::new(0);
    let cancel = || asked.fetch_add(1, Ordering::Relaxed) > 0;
    assert_eq!(run(&Query { text: "web".to_owned(), ..Query::default() }, &clusters, &cancel), None);
    let empty = search("  ", &clusters);
    assert_eq!((empty.hits.len(), empty.total), (0, 0));
    assert!(empty.unread.iter().any(|unread| unread.why == Why::NoStore), "the notes come with no text too");
}
