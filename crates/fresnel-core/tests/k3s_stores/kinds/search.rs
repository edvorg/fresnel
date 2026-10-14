//! The quick search on the stores of a real cluster: weaknesses 5 and 6 of plan section 6.

use fresnel_core::kinds::Kind;
use fresnel_core::search::{self, ClusterSources, Query, Why};
use fresnel_core::store::ClusterStores;

fn find(stores: &ClusterStores, text: &str) -> search::Found {
    let clusters = [ClusterSources { cluster: stores.cluster().clone(), sources: Ok(stores.sources()) }];
    let query = Query { text: text.to_owned(), ..Query::default() };
    search::run(&query, &clusters, &|| false).expect("no cancel")
}

/// The hits as `Kind namespace/name`.
fn hits(found: &search::Found) -> Vec<String> {
    found.hits.iter().map(|hit| format!("{:?} {}/{}", hit.kind, hit.key.namespace, hit.key.name)).collect()
}

/// Weakness 5: the name of a Secret gives a hit, from the store of its metadata.
pub(super) fn a_secret_name_gives_a_hit(stores: &ClusterStores) {
    let found = find(stores, "web-secret");
    assert_eq!(hits(&found).first().map(String::as_str), Some("Secret kinds/web-secret"), "{:?}", hits(&found));
    assert!(found.hits.iter().all(|hit| hit.stale.is_none()), "each store is live");
}

/// Weakness 6: the user cannot list the ConfigMaps. The search names that store with the
/// reason of the server, and the hits of the other kinds stay.
pub(super) fn a_forbidden_kind_is_a_note_and_the_other_hits_stay(stores: &ClusterStores) {
    let found = find(stores, "web");
    let refused: Vec<&search::Unread> =
        found.unread.iter().filter(|unread| matches!(unread.why, Why::Refused(_))).collect();
    let [note] = refused.as_slice() else { panic!("one refused store: {:#?}", found.unread) };
    assert_eq!(note.kind, Some(Kind::ConfigMap));
    assert!(note.why.to_string().starts_with("forbidden: configmaps is forbidden"), "{}", note.why);
    let hits = hits(&found);
    assert!(hits.contains(&"Secret kinds/web-secret".to_owned()), "{hits:?}");
    assert!(!hits.iter().any(|hit| hit.starts_with("ConfigMap ")), "{hits:?}");
}
