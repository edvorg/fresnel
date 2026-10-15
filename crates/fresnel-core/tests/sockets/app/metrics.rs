//! The metrics of the selected clusters in the view of the core: the core starts a sampler
//! with the stores of each cluster, gives its metrics in the data of the cluster, wakes the
//! UI for each poll, and stops the sampler with the disconnect.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::time::Duration;

use fresnel_core::app::{CoreSettings, CoreView};
use fresnel_core::cluster::ClusterId;
use fresnel_core::metrics::{Latest, Metrics, MetricsSettings, PollState, SeriesKey};
use fresnel_testkit::{FakeApiServer, Fault};
use tokio::time::timeout;

use super::{App, home, write_kubeconfig};
use crate::common::in_time;

const PODS: &str = "/apis/metrics.k8s.io/v1beta1/pods";
const NODES: &str = "/apis/metrics.k8s.io/v1beta1/nodes";
/// The time between two polls: short, so that the test sees many polls.
const PERIOD: Duration = Duration::from_millis(200);

/// A NodeMetrics list with the node `node-a`, of this CPU. The item has no window: a window
/// longer than the period of the settings would be the period.
fn node_list(cpu: &str) -> String {
    format!(
        r#"{{"kind":"NodeMetricsList","items":[{{"metadata":{{"name":"node-a"}},
        "timestamp":"2026-10-15T10:00:05Z","usage":{{"cpu":"{cpu}","memory":"1Gi"}}}}]}}"#
    )
}

/// The metrics of a cluster in the view. `None` for a cluster that is not connected.
fn metrics<'a>(view: &'a CoreView, cluster: &str) -> Option<&'a Metrics> {
    let row = view.clusters.iter().find(|row| row.id.as_str() == cluster)?;
    Some(&row.data.as_ref()?.metrics)
}

/// The value of `node-a` in the metrics of `of`, looked up for `cluster`.
fn node_a(view: &CoreView, of: &str, cluster: &str) -> Option<Latest> {
    metrics(view, of)?.latest(&ClusterId::new(cluster), &SeriesKey::Node("node-a".into()))
}

/// Weaknesses 2 and 4 of plan section 6 through the core: two clusters with a node of the
/// same name keep their own values, and a cluster that loses its metrics API shows that
/// state and keeps its old value as old, while the other cluster keeps its values.
#[tokio::test(flavor = "multi_thread")]
async fn each_cluster_has_its_own_metrics_and_one_with_no_metrics_api_changes_no_other() {
    let (ams, fra) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    ams.set_body(NODES, &node_list("250m"));
    fra.set_body(NODES, &node_list("750m"));
    let home = home("metrics");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
    let settings = MetricsSettings { period: PERIOD, ..MetricsSettings::default() };
    let app = App::started(&home, CoreSettings { metrics: settings, ..CoreSettings::default() });
    app.select("fra", true);

    let cpu = |view: &CoreView, cluster: &str| {
        node_a(view, cluster, cluster).filter(|latest| latest.current).map(|latest| latest.sample.usage.cpu)
    };
    let both =
        app.until("a value of each node-a", |view| cpu(view, "ams").is_some() && cpu(view, "fra").is_some()).await;
    assert_eq!((cpu(&both, "ams"), cpu(&both, "fra")), (Some(250_000_000), Some(750_000_000)));
    assert_eq!((node_a(&both, "ams", "fra"), node_a(&both, "fra", "ams")), (None, None), "a lookup of the other");

    // Each poll wakes the UI: the views of five wakes in a row have five counts of answers.
    // With no wake of a poll, the other wakes of the core come seconds apart.
    let counts = RefCell::new(BTreeSet::new());
    let woken = app.until("five polls of ams", |view| {
        let mut counts = counts.borrow_mut();
        counts.extend(metrics(view, "ams").map(|ams| ams.nodes.answers));
        counts.len() >= 5
    });
    timeout(PERIOD * 50, woken).await.unwrap_or_else(|_| panic!("each poll wakes the UI: {:?}", counts.borrow()));

    fra.add_fault(Fault::not_found().on_path(PODS));
    fra.add_fault(Fault::not_found().on_path(NODES));
    let not_installed = |view: &CoreView| {
        metrics(view, "fra").is_some_and(|fra| {
            matches!((&fra.pods.state, &fra.nodes.state), (PollState::NotInstalled(_), PollState::NotInstalled(_)))
        })
    };
    let failed = app.until("no metrics API in fra", not_installed).await;
    let answers = metrics(&failed, "ams").map(|ams| ams.nodes.answers).unwrap_or_default();
    let later = app
        .until("two more answers of ams", |view| {
            not_installed(view) && metrics(view, "ams").is_some_and(|ams| ams.nodes.answers >= answers + 2)
        })
        .await;
    assert_eq!(cpu(&later, "ams"), Some(250_000_000), "ams keeps its values");
    let old = node_a(&later, "fra", "fra").map(|latest| (latest.sample.usage.cpu, latest.current));
    assert_eq!(old, Some((750_000_000, false)), "fra keeps its value, and it is old");

    // The sampler stops with the disconnect: no read of the metrics after it.
    app.select("fra", false);
    app.until("fra is not connected", |view| metrics(view, "fra").is_none()).await;
    in_time("each connection of fra closes", fra.wait_for(|counters| counters.open_connections == 0)).await;
    let reads = || fra.requests().iter().filter(|seen| seen.uri.path().starts_with("/apis/metrics.k8s.io")).count();
    let before = reads();
    // Only a check that nothing occurs waits a fixed time: five periods.
    tokio::time::sleep(PERIOD * 5).await;
    assert_eq!(reads(), before, "no poll after the disconnect");
}
