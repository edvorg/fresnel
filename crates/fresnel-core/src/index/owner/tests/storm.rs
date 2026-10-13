//! The owner index in a storm of changes. A writer changes the four stores as fast as it
//! can, and the task of the index reads them on an other thread at the same time. At the
//! end, the index is the index of one new read of the stores: no difference of two
//! snapshots lost a change, also when one difference holds many changes.

use std::collections::BTreeMap;
use std::time::Duration;

use fastrand::Rng;
use fresnel_net::task::Backoff;
use serde::de::DeserializeOwned;
use tokio::time::timeout;

use super::*;
use crate::cluster::ClusterId;
use crate::index::{IndexParts, Starter};
use crate::published::Notice;

const STEPS: usize = 4_000;
const PODS: usize = 40;
const SETS: usize = 4;
const JOBS: usize = 2;
const SLICES: usize = 4;
const SERVICES: usize = 2;

fn uid(kind: &str, number: usize) -> String {
    format!("uid-{kind}-{number}")
}

/// An object as a watch gives it. `rest` closes the metadata and holds each other field.
fn object<T: DeserializeOwned>(name: &str, uid: &str, version: usize, owners: &[String], rest: &str) -> T {
    let owner = |uid: &String| format!(r#"{{"apiVersion":"v1","kind":"Owner","name":"owner","uid":"{uid}"}}"#);
    let owners = owners.iter().map(owner).collect::<Vec<_>>().join(",");
    let identity = format!(r#""name":"{name}","namespace":"shop","resourceVersion":"{version}","uid":"{uid}""#);
    serde_json::from_str(&format!(r#"{{"metadata":{{{identity},"ownerReferences":[{owners}]{rest}}}"#))
        .expect("an object")
}

/// One store with the objects that the writer gave to it, by their names.
struct Written<T> {
    store: TestStore<T>,
    objects: BTreeMap<String, T>,
}

impl<T: Object + Clone> Written<T> {
    /// A live store with no object.
    fn new(bed: &Bed) -> Self {
        let mut store = bed.store();
        store.list(Vec::new());
        Self { store, objects: BTreeMap::new() }
    }

    fn apply(&mut self, name: String, object: T) {
        self.store.apply(object.clone());
        self.objects.insert(name, object);
    }

    fn delete(&mut self, name: &str) {
        if let Some(object) = self.objects.remove(name) {
            self.store.delete(&object);
        }
    }

    /// A new list, as after a break of the watch in which this object left the cluster.
    fn list_again(&mut self, gone: &str) {
        self.objects.remove(gone);
        self.store.list(self.objects.values().cloned().collect());
    }
}

struct Storm {
    random: Rng,
    sets: Written<ReplicaSet>,
    jobs: Written<Job>,
    pods: Written<Pod>,
    slices: Written<EndpointSlice>,
}

impl Storm {
    fn indexer(&self) -> OwnerIndexer {
        let (sets, jobs, pods, slices) = (&self.sets.store, &self.jobs.store, &self.pods.store, &self.slices.store);
        OwnerIndexer::new(sets.reader(), jobs.reader(), pods.reader(), slices.reader())
    }

    /// One change of one store. `version` is new for each step.
    fn step(&mut self, version: usize) {
        let mut pick = |count: usize| self.random.usize(..count);
        match pick(20) {
            // A pod: a new one, a new status, new owners, or a new pod with the name of an old one.
            0..=8 => {
                let (number, set, job) = (pick(PODS), uid("set", pick(SETS)), uid("job", pick(JOBS)));
                let owners =
                    [vec![], vec![set.clone()], vec![job.clone()], vec![set.clone(), job], vec![set.clone(), set]];
                let (name, uid) = (format!("pod-{number}"), format!("uid-pod-{number}-{}", pick(2)));
                self.pods.apply(name.clone(), object(&name, &uid, version, &owners[pick(5)], "}"));
            }
            9..=10 => self.pods.delete(&format!("pod-{}", pick(PODS))),
            11 => self.pods.list_again(&format!("pod-{}", pick(PODS))),
            12..=13 => {
                let (number, owner) = (pick(SETS), uid("deployment", pick(2)));
                let set = object(&format!("set-{number}"), &uid("set", number), version, &[owner], "}");
                self.sets.apply(format!("set-{number}"), set);
            }
            14 => self.sets.list_again(&format!("set-{}", pick(SETS))),
            // A Job of the CronJob, or a Job that some Job owns: an owner loop comes and goes.
            15..=16 => {
                let number = pick(JOBS);
                let owner = [uid("cron", 0), uid("job", pick(JOBS))][pick(2)].clone();
                let job = object(&format!("job-{number}"), &uid("job", number), version, &[owner], "}");
                self.jobs.apply(format!("job-{number}"), job);
            }
            17 => self.slices.delete(&format!("slice-{}", pick(SLICES))),
            _ => {
                let (number, service, pod, ready) = (pick(SLICES), pick(SERVICES), pick(PODS), pick(2) == 0);
                let endpoint =
                    format!(r#"{{"conditions":{{"ready":{ready}}},"targetRef":{{"kind":"Pod","name":"pod-{pod}"}}}}"#);
                let labels = format!(r#""labels":{{"kubernetes.io/service-name":"svc-{service}"}}"#);
                let rest = format!(r#",{labels}}},"endpoints":[{endpoint}]"#);
                let slice = object(&format!("slice-{number}"), &uid("slice", number), version, &[], &rest);
                self.slices.apply(format!("slice-{number}"), slice);
            }
        }
    }

    /// The mark of the end: a state that only the last snapshot of each store has.
    fn stop(&mut self) {
        self.sets.store.set_state(StoreState::Stopped);
        self.jobs.store.set_state(StoreState::Stopped);
        self.pods.store.set_state(StoreState::Stopped);
        self.slices.store.set_state(StoreState::Stopped);
    }
}

/// Each answer of the index for each owner and each Service of the storm.
fn answers(index: &Index<Owners>) -> Vec<String> {
    let owners = [("deployment", 2), ("cron", 1), ("set", SETS), ("job", JOBS)];
    let owners = owners.iter().flat_map(|(kind, count)| (0..*count).map(|number| uid(kind, number)));
    let below = owners.map(|owner| {
        format!("{owner}: {:?} {:?}", index.owned(Kind::Deployment, &owner), index.pods_of(Kind::Deployment, &owner))
    });
    let services = (0..SERVICES).map(|number| ObjectKey::new("shop", &format!("svc-{number}")));
    let behind = services.map(|service| {
        format!("{}: {:?} {:?}", service.name, index.slices_of(&service), index.pods_of_service(&service))
    });
    below.chain(behind).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_index_after_a_storm_of_changes_is_the_index_of_a_new_read_of_the_stores() {
    let bed = Bed::new();
    let (sets, jobs, pods, slices) = (Written::new(&bed), Written::new(&bed), Written::new(&bed), Written::new(&bed));
    let mut storm = Storm { random: Rng::with_seed(2026), sets, jobs, pods, slices };
    let (cluster, parts) = (ClusterId::new("storm"), IndexParts { bin: bed.bin(), restart: Backoff::default() });
    let starter = Starter { tasks: &bed.tasks, cluster: &cluster, parts, notice: Notice::new() };
    let running = starter.start("owner index", {
        let (sets, jobs) = (storm.sets.store.reader(), storm.jobs.store.reader());
        let (pods, slices) = (storm.pods.store.reader(), storm.slices.store.reader());
        move || OwnerIndexer::new(sets.clone(), jobs.clone(), pods.clone(), slices.clone())
    });

    // A second index reads in the thread of the writer, each few steps: its differences
    // hold some changes, in a sequence that is the same in each run of the test.
    let mut stepwise = storm.indexer();
    for version in 1..=STEPS {
        storm.step(version);
        if storm.random.usize(..8) == 0 {
            read(&mut stepwise);
        }
    }
    storm.stop();

    let mut index = running.publisher.reader();
    let at_rest = async {
        loop {
            let read = index.read();
            if read.sources().len() == 4 && read.sources().iter().all(|source| source.state == StoreState::Stopped) {
                return read;
            }
            index.changed().await;
        }
    };
    let of_task = timeout(Duration::from_secs(30), at_rest).await.expect("the task reads the last snapshots in time");
    let (of_new_read, _) = read(&mut storm.indexer());
    let expected = answers(&of_new_read);
    assert!(expected.iter().filter(|answer| answer.contains("pod-")).count() >= 6, "{expected:#?}");
    assert_eq!(answers(&read(&mut stepwise).0), expected, "the index of the reads between the steps");
    assert_eq!(answers(&of_task), expected, "the index of the task");
    assert!(bed.reports.try_recv().is_err(), "no task failed");
}
