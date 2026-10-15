//! The metrics sampler on the fake API server: a poll reads each list one time through the
//! gateway, with no retry, and each refusal of the server gives the state of its list.

use std::time::Duration;

use fresnel_core::metrics::{Inputs, Metrics, MetricsSettings, PollState, Sampler, SamplerParts, SeriesKey};
use fresnel_core::net::{Limits, NetSettings, WallClock};
use fresnel_core::published::{Disposal, Notice};
use fresnel_core::store::StoreSettings;
use fresnel_testkit::Fault;
use tokio::time::timeout;

use crate::bed::Bed;
use crate::common::LIMIT;
use crate::store_bed::Stores;

const PODS: &str = "/apis/metrics.k8s.io/v1beta1/pods";
const NODES: &str = "/apis/metrics.k8s.io/v1beta1/nodes";
const NODE_LIST: &str = r#"{"kind":"NodeMetricsList","items":[{"metadata":{"name":"node-a"},
    "timestamp":"2026-10-15T10:00:05Z","window":"15s","usage":{"cpu":"250m","memory":"1Gi"}}]}"#;

/// One cluster on the fake server with its stores, and a sampler for each check. The period
/// of a sampler is longer than the test, so each sampler polls one time.
struct Polls {
    stores: Stores,
    disposal: Disposal,
    bed: Bed,
}

impl Polls {
    async fn start() -> Self {
        // A read with no answer fails after 1 s, not after the 10 s of the app.
        let limits = Limits { get: Duration::from_secs(1), ..Limits::default() };
        let bed = Bed::start(NetSettings { limits, ..NetSettings::default() }).await;
        let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
        Self { stores, disposal: Disposal::start(&bed.tasks.tasks), bed }
    }

    /// The metrics of the first poll of a new sampler, and the count of reads of each list
    /// that the poll sent.
    async fn first_poll(&self) -> (Metrics, usize, usize) {
        let reads = |path: &str| self.bed.server.requests().iter().filter(|seen| seen.uri.path() == path).count();
        let before = (reads(PODS), reads(NODES));
        let settings = MetricsSettings { period: Duration::from_secs(3_600), ..MetricsSettings::default() };
        let parts = SamplerParts { settings, bin: self.disposal.bin(), notice: Notice::new(), clock: WallClock::system() };
        let indexes = self.stores.indexes(&self.bed.tasks);
        let inputs = Inputs::of(&self.stores.stores, &indexes).expect("a store of each kind");
        let sampler = Sampler::start(&self.bed.tasks.tasks, self.bed.link.gateway.clone(), inputs, parts);
        let mut reader = sampler.reader();
        let polled = async {
            loop {
                let metrics = Metrics::clone(&reader.read());
                if metrics.next.is_some() {
                    return metrics;
                }
                reader.changed().await;
            }
        };
        let metrics = timeout(LIMIT, polled).await.expect("in time: the first poll");
        (metrics, reads(PODS) - before.0, reads(NODES) - before.1)
    }
}

#[tokio::test]
async fn a_poll_reads_each_list_one_time_and_each_refusal_gives_the_state_of_its_list() {
    let polls = Polls::start().await;
    let server = &polls.bed.server;
    server.add_fault(Fault::unavailable().on_path(PODS));
    server.add_fault(Fault::not_found().on_path(NODES));
    let (metrics, pods, nodes) = polls.first_poll().await;
    assert!(matches!(metrics.pods.state, PollState::Unavailable(_)), "{}", metrics.pods.state);
    assert_eq!(metrics.pods.state.to_string(), "metrics-server not available: the server answered HTTP 503");
    assert!(matches!(metrics.nodes.state, PollState::NotInstalled(_)), "{}", metrics.nodes.state);
    assert_eq!((pods, nodes), (1, 1), "a 503 and a 404 get no retry");

    server.clear_faults();
    server.add_fault(Fault::forbidden().on_path(PODS));
    server.add_fault(Fault::no_answer().on_path(NODES));
    let (metrics, pods, nodes) = polls.first_poll().await;
    assert!(matches!(metrics.pods.state, PollState::Forbidden(_)), "{}", metrics.pods.state);
    assert!(matches!(metrics.nodes.state, PollState::Timeout(_)), "{}", metrics.nodes.state);
    assert_eq!((pods, nodes), (1, 1), "a read with no answer ends at its limit, with no retry");

    server.clear_faults();
    server.set_body(NODES, NODE_LIST);
    let (metrics, ..) = polls.first_poll().await;
    assert_eq!((metrics.pods.state.is_ok(), metrics.nodes.state.is_ok()), (true, true));
    let cluster = metrics.cluster.clone();
    let node = metrics.latest(&cluster, &SeriesKey::Node("node-a".into())).expect("a value of the node");
    assert_eq!((node.sample.usage.cpu, node.sample.usage.memory, node.current), (250_000_000, 1 << 30, true));
}
