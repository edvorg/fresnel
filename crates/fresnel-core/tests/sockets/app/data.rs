//! The objects of the selected clusters in the view of the core.

use fresnel_core::app::{ClusterData, CoreSettings, CoreView, Health};
use fresnel_core::index::Basis;
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::FakeApiServer;

use super::{App, home, row, write_kubeconfig};
use crate::common::in_time;
use crate::stores::PODS;

fn pod(name: &str, version: u32) -> String {
    format!(r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"u-{name}","resourceVersion":"{version}"}}}}"#)
}

fn list(items: &[String]) -> String {
    format!(r#"{{"metadata":{{"resourceVersion":"7"}},"items":[{}]}}"#, items.join(","))
}

/// The data of a cluster in the view. `None` before the start gave the clusters, and for
/// a cluster that is not connected.
fn data<'a>(view: &'a CoreView, cluster: &str) -> Option<&'a ClusterData> {
    view.clusters.iter().find(|row| row.id.as_str() == cluster)?.data.as_ref()
}

/// The names of the pods of a cluster in the view. `None` while its Pod store is not live.
fn live_pods(view: &CoreView, cluster: &str) -> Option<Vec<String>> {
    let pods = &data(view, cluster)?.pods;
    pods.state.is_live().then(|| pods.items.iter().map(|(key, _)| key.name.to_string()).collect())
}

/// Design rule 8 and weakness 1 of plan section 6: each selected cluster gives its own
/// objects, and a cluster that leaves the selection gives none.
#[tokio::test(flavor = "multi_thread")]
async fn the_view_has_the_pods_and_the_events_of_each_selected_cluster_and_a_change_wakes_the_ui() {
    let (ams, fra) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    ams.set_body(PODS, &list(&[pod("api", 5), pod("web", 6)]));
    fra.set_body(PODS, &list(&[pod("db", 5)]));
    let about_api = r#"{"metadata":{"name":"api.1","namespace":"shop","resourceVersion":"3"},"reason":"Pulled",
        "involvedObject":{"kind":"Pod","namespace":"shop","name":"api","uid":"u-api"}}"#;
    ams.set_body(&Kind::Event.info().path(), &list(&[about_api.to_owned()]));
    let home = home("data");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
    let app = App::started(&home, CoreSettings::default());

    let one = app.until("the pods of ams", |view| live_pods(view, "ams").is_some()).await;
    assert_eq!(live_pods(&one, "ams"), Some(vec!["api".to_owned(), "web".to_owned()]));
    assert!(row(&one, "fra").data.is_none(), "a cluster that is not selected has no store");
    assert_eq!(fra.counters().connections, 0);

    app.select("fra", true);
    let two = app.until("the pods of fra", |view| live_pods(view, "fra").is_some()).await;
    assert_eq!(live_pods(&two, "fra"), Some(vec!["db".to_owned()]));
    assert_eq!(live_pods(&two, "ams"), Some(vec!["api".to_owned(), "web".to_owned()]), "each cluster has its own");

    // The Events of one object, with what the answer is good for.
    let events_of_api = |view: &CoreView| {
        let index = data(view, "ams").and_then(|data| data.events.as_ref()).expect("the Event index");
        index.events_of(Kind::Pod, &ObjectKey::new("shop", "api"), "u-api")
    };
    let with_events = app.until("the Events of the pod", |view| events_of_api(view).basis == Basis::Current).await;
    let reasons: Vec<Option<String>> =
        events_of_api(&with_events).found.iter().map(|event| event.reason.clone()).collect();
    assert_eq!(reasons, [Some("Pulled".to_owned())]);

    // A change of the cluster wakes the UI, and the next view has it.
    let added = format!(r#"{{"type":"ADDED","object":{}}}"#, pod("cache", 8));
    assert!(fra.send_event(PODS, &added) > 0, "the watch of the pods is open");
    let grown = app.until("the new pod", |view| live_pods(view, "fra").is_some_and(|pods| pods.len() == 2)).await;
    assert_eq!(live_pods(&grown, "fra"), Some(vec!["cache".to_owned(), "db".to_owned()]));
    assert_eq!(live_pods(&two, "fra"), Some(vec!["db".to_owned()]), "a view that a reader holds does not change");

    // A cluster that leaves the selection stops its stores, and its connections close.
    app.select("fra", false);
    let left = app.until_health("fra", Health::Off).await;
    assert!(row(&left, "fra").data.is_none());
    in_time("each watch of fra closes", fra.wait_for(|counters| counters.open_watches == 0)).await;
    in_time("each connection of fra closes", fra.wait_for(|counters| counters.open_connections == 0)).await;
    assert_eq!(live_pods(&grown, "fra").map(|pods| pods.len()), Some(2), "the copy of a reader keeps its objects");

    app.core.stop();
    in_time("the stop closes each watch", ams.wait_for(|counters| counters.open_watches == 0)).await;
    in_time("the stop closes each connection", ams.wait_for(|counters| counters.open_connections == 0)).await;
}

/// A cluster with no stores would show an empty list in place of an error (design rule
/// 5). So stores that do not start are a connect that failed, with the reason.
#[tokio::test(flavor = "multi_thread")]
async fn stores_that_do_not_start_are_a_failed_connect_with_the_reason() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("no-stores");
    write_kubeconfig(&home, "fake", &[("fake", server.addr(), &server, false)]);
    let mut settings = CoreSettings::default();
    settings.stores.watch.page_size = 0;
    let app = App::started(&home, settings);

    let view = app.until_health("fake", Health::Failed).await;
    let failed = row(&view, "fake");
    assert_eq!(failed.reason().as_deref(), Some("a list page has one object at least"));
    assert!(failed.data.is_none());
    let texts = app.until_texts("fake", |texts| !texts.is_empty()).await;
    assert_eq!(texts, ["the connect did not start: a list page has one object at least"]);
    in_time("the connections close", server.wait_for(|counters| counters.open_connections == 0)).await;
    assert!(server.requests().iter().all(|seen| seen.uri.path() == "/version"), "no store sent a request");
}
