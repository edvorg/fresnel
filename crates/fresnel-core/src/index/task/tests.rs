use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fresnel_net::task::{Backoff, End};

use super::*;
use crate::cluster::ClusterId;
use crate::index::bed::{Bed, EPOCH, TestStore};
use crate::index::source::Tracked;
use crate::index::{IndexParts, Starter};
use crate::kinds::{Kind, Meta};
use crate::published::{Notice, Reader};
use crate::store::{Change, StoreState, Wait};

/// An index for the tests of the task: the names of the objects of one store. Its read
/// panics while the flag of the test is set.
struct Names {
    names: Vec<String>,
    store: Tracked<Meta>,
    panics: Arc<AtomicBool>,
}

impl Indexer for Names {
    type Relations = Vec<String>;

    fn read(&mut self) -> Read {
        assert!(!self.panics.load(Ordering::Relaxed), "a fault of the test");
        let names = &mut self.names;
        let mut changed = false;
        let source = self.store.read(|change| {
            changed = true;
            match change {
                Change::Added(key, _) => names.push(key.name.to_string()),
                Change::Removed(key, _) => names.retain(|name| **name != *key.name),
                Change::Updated { .. } => changed = false,
            }
        });
        Read { changed, sources: vec![source] }
    }

    fn relations(&self) -> Vec<String> {
        self.names.clone()
    }

    async fn changed(&mut self) {
        self.store.changed().await;
    }
}

fn object(name: &str, version: &str) -> Meta {
    let text = format!(
        r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"uid-{name}","resourceVersion":"{version}"}}}}"#
    );
    serde_json::from_str(&text).expect("an object")
}

/// A started index of the names of one store, with a reader of it and the notice of its
/// new values.
struct Started {
    bed: Bed,
    store: TestStore<Meta>,
    panics: Arc<AtomicBool>,
    index: Reader<Index<Vec<String>>>,
    notice: Notice,
    running: crate::index::Running<Vec<String>>,
}

fn start() -> Started {
    let bed = Bed::new();
    let store = bed.store::<Meta>();
    let panics = Arc::new(AtomicBool::new(false));
    let restart =
        Backoff { first: Duration::from_secs(1), max: Duration::from_secs(1), jitter_percent: 0, ..Backoff::default() };
    let notice = Notice::new();
    let (cluster, parts) = (ClusterId::new("test"), IndexParts { bin: bed.bin(), restart });
    let starter = Starter { tasks: &bed.tasks, cluster: &cluster, parts, notice: notice.clone() };
    let running = starter.start("names index", {
        let (reader, panics) = (store.reader(), Arc::clone(&panics));
        move || Names {
            names: Vec::new(),
            store: Tracked::new(Kind::ConfigMap, reader.clone()),
            panics: Arc::clone(&panics),
        }
    });
    let index = running.publisher.reader();
    Started { bed, store, panics, index, notice, running }
}

impl Started {
    /// Waits for the next value of the index.
    async fn next(&mut self) -> Index<Vec<String>> {
        self.index.changed().await;
        (*self.index.read()).clone()
    }
}

#[tokio::test(start_paused = true)]
async fn the_readers_get_a_new_index_for_a_new_relation_and_for_a_new_fact_of_a_store() {
    let mut started = start();
    let first = started.index.read();
    assert_eq!((first.generation, &first.state, first.sources().len()), (0, &IndexState::Starting, 0));
    assert_eq!(first.basis(&[Kind::ConfigMap]), Basis::NotFollowed(IndexState::Starting), "no answer is good yet");

    // The first read: no object, and the store had no list.
    let read = started.next().await;
    let waiting =
        SourceStatus { kind: Kind::ConfigMap, listed: false, state: StoreState::Waiting(Wait::Channel), since: EPOCH };
    assert_eq!((read.generation, &read.state, read.sources()), (0, &IndexState::Following, &[waiting.clone()][..]));
    assert_eq!(read.basis(&[Kind::ConfigMap]), Basis::NotLoaded(waiting));

    started.store.list(vec![object("a", "1"), object("b", "1")]);
    let listed = started.next().await;
    assert_eq!((listed.generation, listed.relations().as_slice()), (1, &["a".to_owned(), "b".to_owned()][..]));
    assert_eq!(listed.basis(&[Kind::ConfigMap]), Basis::Current);
    assert!(listed.basis(&[Kind::ConfigMap]).is_loaded());

    // A new fact of the store is a new value with the same relations.
    let mut changes = started.notice.changes();
    started.store.set_state(StoreState::Waiting(Wait::Stream));
    let old = started.next().await;
    assert_eq!(old.generation, 1, "no relation changed");
    let Basis::Old(source) = old.basis(&[Kind::ConfigMap]) else { panic!("the store is not live") };
    assert_eq!((source.listed, &source.state), (true, &StoreState::Waiting(Wait::Stream)));
    assert!(old.basis(&[Kind::ConfigMap]).is_loaded() && !Basis::NotLoaded(source).is_loaded());
    changes.next().await;

    // A snapshot with no new relation and no new fact gives no new value: a bookmark,
    // and a new version of an object.
    started.store.bookmark();
    started.store.apply(object("a", "2"));
    started.store.delete(&object("b", "1"));
    let last = started.next().await;
    assert_eq!((last.generation, last.relations().as_slice()), (2, &["a".to_owned()][..]), "one value for the delete");
    assert!(tokio::time::timeout(Duration::from_secs(60), changes.next()).await.is_ok());
    assert!(tokio::time::timeout(Duration::from_secs(60), changes.next()).await.is_err(), "one notice only");
}

/// Design rule 1 and plan section 13.5: a panic of a task gives a state, a journal entry
/// and a restart.
#[tokio::test(start_paused = true)]
async fn a_panic_of_the_task_shows_as_the_state_of_the_index_and_the_task_starts_again() {
    let mut started = start();
    started.store.list(vec![object("a", "1")]);
    while started.next().await.generation == 0 {}

    started.panics.store(true, Ordering::Relaxed);
    started.store.apply(object("b", "1"));
    let failed = started.next().await;
    let reason = "the task of the index panicked: a fault of the test";
    assert_eq!(failed.state, IndexState::Failed { reason: reason.into() });
    assert_eq!((failed.generation, failed.relations().as_slice()), (1, &["a".to_owned()][..]), "the index before");
    assert_eq!(failed.basis(&[Kind::ConfigMap]), Basis::NotFollowed(failed.state.clone()));
    let report = started.bed.reports.recv_timeout(Duration::from_secs(5)).expect("the report for the journal");
    assert_eq!(report.task.to_string(), "names index [test]");
    assert!(matches!(&report.end, End::Panicked(message) if message == "a fault of the test"), "{:?}", report.end);
    assert_eq!(report.restart_in, Some(Duration::from_secs(1)));

    // The next run builds each relation again, and the count of the changes goes on.
    started.panics.store(false, Ordering::Relaxed);
    let again = started.next().await;
    assert_eq!(again.state, IndexState::Following);
    assert_eq!((again.generation, again.relations().as_slice()), (2, &["a".to_owned(), "b".to_owned()][..]));
}

#[tokio::test(start_paused = true)]
async fn a_stopped_index_keeps_its_relations_and_ends_its_task() {
    let mut started = start();
    started.store.list(vec![object("a", "1")]);
    while started.next().await.generation == 0 {}
    let names = |bed: &Bed| bed.tasks.snapshot().iter().map(|task| task.name.to_string()).collect::<Vec<_>>();
    assert!(names(&started.bed).contains(&"names index [test]".to_owned()));

    drop(started.running);
    let stopped = started.index.read();
    assert_eq!((&stopped.state, stopped.relations().as_slice()), (&IndexState::Stopped, &["a".to_owned()][..]));
    assert_eq!(stopped.basis(&[Kind::ConfigMap]), Basis::NotFollowed(IndexState::Stopped));
    assert_eq!(names(&started.bed), ["old snapshots"]);
    assert!(started.bed.reports.try_recv().is_err(), "a stop is no fault");
}
