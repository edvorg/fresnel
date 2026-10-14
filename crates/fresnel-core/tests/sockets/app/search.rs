//! The quick search on the stores of a real core (plan section 12.1): each store that is
//! always on is a source, and a store that the server refuses is a note.

use fresnel_core::app::{CoreSettings, CoreView};
use fresnel_core::kinds::Kind;
use fresnel_core::search::{self, Query, Why};
use fresnel_core::store::StoreState;
use fresnel_testkit::{FakeApiServer, Fault};

use super::{App, home, write_kubeconfig};

fn list(items: &[&str]) -> String {
    format!(r#"{{"metadata":{{"resourceVersion":"7"}},"items":[{}]}}"#, items.join(","))
}

const SECRET: &str = r#"{"metadata":{"name":"web-secret","namespace":"shop","uid":"u-s","resourceVersion":"4"}}"#;

/// True when the two clusters are selected and their Secrets are no longer on their way.
fn secrets_settled(view: &CoreView) -> bool {
    let selected: Vec<_> = view.clusters.iter().filter(|row| row.selected).collect();
    selected.len() == 2
        && selected.into_iter().all(|row| {
            let state = row.data.as_ref().and_then(|data| data.store(Kind::Secret)).map(|status| &status.state);
            matches!(state, Some(StoreState::Live | StoreState::Forbidden { .. }))
        })
}

/// Weaknesses 5 and 6 of plan section 6 on the stores of the core: the Secret of one
/// cluster is a hit, and the Secrets of the other one are a note with the reason.
#[tokio::test(flavor = "multi_thread")]
async fn each_store_of_each_selected_cluster_is_a_source_and_a_forbidden_one_is_a_note() {
    let (ams, fra) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    let secrets = Kind::Secret.info().path();
    ams.set_body(&secrets, &list(&[SECRET]));
    fra.add_fault(Fault::forbidden().on_path(secrets.clone()));
    let home = home("search");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until("the selection of ams", |view| view.clusters.iter().any(|row| row.data.is_some())).await;
    app.select("fra", true);
    let view = app.until("the Secrets of each cluster", secrets_settled).await;

    let sources: Vec<_> = view.clusters.iter().filter(|row| row.selected).map(|row| row.search_sources()).collect();
    for cluster in &sources {
        let kinds: Vec<Kind> =
            cluster.sources.as_ref().expect("stores").iter().map(|source| source.status.kind).collect();
        let always: Vec<Kind> = Kind::ALL.iter().copied().filter(|kind| kinds.contains(kind)).collect();
        assert_eq!(kinds, always, "{}: one source for each store that is always on", cluster.cluster.as_str());
        assert!(!kinds.contains(&Kind::Lease) && kinds.contains(&Kind::Secret) && kinds.contains(&Kind::Pod));
    }
    let query = Query { text: "web-secret".to_owned(), ..Query::default() };
    let found = search::run(&query, &sources, &|| false).expect("no cancel");
    let hits: Vec<(&str, Kind, &str)> =
        found.hits.iter().map(|hit| (hit.cluster.as_str(), hit.kind, &*hit.key.name)).collect();
    assert_eq!(hits, [("ams", Kind::Secret, "web-secret")]);
    let refused = found.unread.iter().find(|unread| matches!(unread.why, Why::Refused(_)));
    let refused = refused.map(|unread| (unread.cluster.as_str(), unread.kind));
    assert_eq!(refused, Some(("fra", Some(Kind::Secret))), "{:#?}", found.unread);
}
