//! The limits of the history on a real cluster: the memory limit, the time of each tier,
//! and the drop of the series of a deleted pod. Each check runs one more sampler of `b`
//! with settings of its own, next to the sampler with the defaults.

use std::time::Duration;

use fresnel_core::kinds::Kind;
use fresnel_core::metrics::{Metrics, MetricsSettings, SeriesKey};
use fresnel_core::store::ObjectKey;
use serde_json::json;
use tokio::time::Instant;

use crate::bed::{Bed, until};

/// The history of a new sampler grows past the size of a probe whose series each hold one
/// chunk; the limit then drops the oldest samples of the containers first. A tier holds its
/// samples in chunks of 64 from its third sample on, so the size of a history grows by a
/// chunk, not by a sample: the limit is one byte below the size of the probe.
pub(crate) async fn the_memory_limit_holds_and_the_containers_lose_first(bed: &Bed) {
    let b = &bed.b;
    let text = b
        .kubectl(&["get", "pods", "--namespace", "kube-system", "--selector", "k8s-app=kube-dns", "--output", "name"])
        .await;
    let coredns = ObjectKey::new("kube-system", text.trim().trim_start_matches("pod/"));
    let of_cluster =
        |metrics: &Metrics| metrics.series(&b.id, &SeriesKey::Cluster).map_or(0, |series| series.samples().count());
    let of_coredns =
        |metrics: &Metrics| metrics.containers(&b.id, &coredns).map(|(_, series)| series.samples().count()).next();
    let chunked = |metrics: &Metrics| of_cluster(metrics) >= 3 && of_coredns(metrics).is_some_and(|count| count >= 3);
    let probe = bed.sampler(b, MetricsSettings::default());
    let limit = until(&probe.reader(), "three samples of the cluster and of coredns", chunked).await.bytes() - 1;
    drop(probe);
    let small = bed.sampler(b, MetricsSettings { memory_limit: limit, ..MetricsSettings::default() });
    let mut reader = small.reader();
    let trimmed = until(&reader, "a sample that the limit dropped", |metrics| metrics.dropped() > 0).await;
    for _ in 0..3 {
        reader.changed().await;
        let metrics = Metrics::clone(&reader.read());
        assert!(metrics.bytes() <= limit, "{} bytes of {limit}", metrics.bytes());
    }
    let container = of_coredns(&trimmed);
    assert!(container.is_some(), "the series of the container of coredns");
    assert!(of_cluster(&trimmed) > container.unwrap_or_default(), "the containers lose their samples first");
}

/// A sampler with a fine tier of 30 s, a coarse tier of 90 s in steps of 30 s, and 20 s for
/// the series of an object that left its store.
pub(crate) async fn the_tiers_keep_their_time_and_the_series_of_a_deleted_pod_go(bed: &Bed) {
    let b = &bed.b;
    let settings = MetricsSettings {
        fine: Duration::from_secs(30),
        step: Duration::from_secs(30),
        coarse: Duration::from_secs(90),
        keep_gone: Duration::from_secs(20),
        ..MetricsSettings::default()
    };
    let short = bed.sampler(b, settings);
    let reader = short.reader();
    let pause = json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "brief"}, "spec": {"terminationGracePeriodSeconds": 0,
        "containers": [{"name": "c", "image": "rancher/mirrored-pause:3.6", "imagePullPolicy": "IfNotPresent"}]}});
    b.create(Kind::Pod, &pause).await;
    let brief = SeriesKey::Pod(ObjectKey::new("default", "brief"));
    let current = |metrics: &Metrics| metrics.latest(&b.id, &brief).is_some_and(|latest| latest.current);
    until(&reader, "a value of the pod brief", current).await;
    b.kubectl(&["delete", "pod", "brief", "--namespace", "default"]).await;
    let deleted = Instant::now();
    until(&reader, "the series of the deleted pod go", |metrics| metrics.series(&b.id, &brief).is_none()).await;
    let after = deleted.elapsed();
    assert!(after >= Duration::from_secs(20) && after < Duration::from_secs(60), "the series went after {after:?}");
    assert!(b.metrics.read().series(&b.id, &brief).is_some(), "the sampler with the defaults keeps it for one hour");

    // Ten answers are 150 s, more than the coarse tier covers.
    let answers = reader.read().nodes.answers;
    let full = until(&reader, "ten more answers", |metrics| metrics.nodes.answers >= answers + 10).await;
    let times: Vec<u32> =
        full.series(&b.id, &SeriesKey::Cluster).expect("the cluster").samples().map(|sample| sample.at).collect();
    let (oldest, newest) = (times.first().copied().unwrap_or_default(), times.last().copied().unwrap_or_default());
    assert!(newest - oldest < 90, "the coarse tier covers 90 s: {times:?}");
    // At most three samples in the fine tier of 30 s, and three steps of 30 s before them.
    assert!((2..=6).contains(&times.len()), "{times:?}");
}
