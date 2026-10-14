//! The objects of the selected clusters in the view of the core.

use std::collections::BTreeSet;

use fresnel_core::app::{ClusterData, Command, CoreSettings, CoreView, Health, ManifestRequest, ManifestState};
use fresnel_core::cluster::ClusterId;
use fresnel_core::index::Basis;
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::ManagedFields;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::FakeApiServer;
use k8s_openapi::api::core::v1::ConfigMap;

use super::selection::until_saved;
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

    until_saved(&home, &["ams"]).await;
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

/// The text of the last manifest read, or its error. `None` while the read is in progress.
fn manifest_of(view: &CoreView, number: u64) -> Option<Result<String, String>> {
    let read = view.manifest.as_ref().filter(|read| read.number == number)?;
    match &read.state {
        ManifestState::Reading => None,
        ManifestState::Done(manifest) => Some(Ok(manifest.text.clone())),
        ManifestState::Failed(error) => Some(Err(error.to_string())),
    }
}

/// Plan section 9.5 and 12.9: a view that shows a tier B kind gets the full objects of it
/// while it shows it, and the read of a manifest gives the YAML of one object.
#[tokio::test(flavor = "multi_thread")]
async fn a_shown_kind_gives_its_full_objects_and_a_manifest_read_gives_the_yaml_of_one_object() {
    let server = FakeApiServer::start().await.expect("a server");
    let config = r#"{"metadata":{"name":"web","namespace":"shop","uid":"u-web","resourceVersion":"5",
        "managedFields":[{"manager":"kubectl"}]},"data":{"mode":"fast"}}"#;
    server.set_body(&Kind::ConfigMap.info().path(), &list(&[config.to_owned()]));
    server.set_body("/api/v1/namespaces/shop/configmaps/web", config);
    let home = home("shown");
    write_kubeconfig(&home, "fake", &[("fake", server.addr(), &server, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until_health("fake", Health::Live).await;

    let mode = |view: &CoreView| {
        let configs = data(view, "fake")?.snapshot::<ConfigMap>()?;
        let web = configs.items.get(&ObjectKey::new("shop", "web"))?;
        web.data.as_ref()?.get("mode").cloned()
    };
    assert_eq!(mode(&app.core.view()), None, "no view shows the ConfigMaps, and a store of metadata has no data");
    app.core.send(Command::ShowKinds(BTreeSet::from([Kind::ConfigMap])));
    let shown = app.until("the full ConfigMaps", |view| mode(view).is_some()).await;
    assert_eq!(mode(&shown).as_deref(), Some("fast"));

    let request = |cluster: &str, managed_fields| ManifestRequest {
        cluster: ClusterId::new(cluster),
        kind: Kind::ConfigMap,
        key: ObjectKey::new("shop", "web"),
        managed_fields,
    };
    for (number, managed_fields) in [(1, ManagedFields::Removed), (2, ManagedFields::Kept)] {
        app.core.send(Command::ReadManifest(request("fake", managed_fields)));
        let read = app.until("the manifest", |view| manifest_of(view, number).is_some()).await;
        let text = manifest_of(&read, number).expect("a read").expect("the manifest");
        assert!(text.contains("name: web") && text.contains("mode: fast"), "{text}");
        assert_eq!(text.contains("managedFields"), managed_fields == ManagedFields::Kept, "{text}");
    }
    app.core.send(Command::ReadManifest(request("absent", ManagedFields::Removed)));
    let failed = app.until("the failed read", |view| manifest_of(view, 3).is_some()).await;
    assert_eq!(manifest_of(&failed, 3), Some(Err("absent is not connected".to_owned())));

    // A view that shows no kind: the view has no full objects of it.
    app.core.send(Command::ShowKinds(BTreeSet::new()));
    app.until("no full ConfigMaps", |view| data(view, "fake").is_some_and(|data| data.objects.kinds().count() == 0))
        .await;
}
