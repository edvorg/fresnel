//! The quick search (plan section 12.1): a key opens it from each page, it lists the hits of
//! each selected cluster with their cluster, kind, namespace and name, and a note for each
//! store that it did not read.
//!
//! - `weakness`: the weaknesses 5 and 6 of plan section 6. `dialog`: the keys, the filters,
//!   the marks and a hit that opens its object. `time`: 4 clusters of made-up rows.
//! - `races`: fast keys and new data while the worker computes.

mod dialog;
mod races;
mod time;
mod weakness;

use std::sync::Arc;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ClusterRow;
use fresnel_core::kinds::{Kind, Meta};
use fresnel_core::search::Source;
use fresnel_core::store::{Snapshot, StoreState};
use k8s_openapi::api::core::v1::Secret;

use crate::data::{empty_store, held, live, of_kinds, pods, store};
use crate::fake::{Window, now, until};

/// The Secrets of the scene of the kinds test: `kinds/web-secret`.
pub(crate) fn secrets() -> Vec<Arc<Secret>> {
    of_kinds()
}

/// The metadata of objects, as the store of a kind of tier B holds it.
pub(crate) fn metadata<K: fresnel_core::kinds::Describe>(objects: &[Arc<K>]) -> Vec<Arc<Meta>> {
    objects.iter().map(|object| Arc::new(Meta { metadata: object.metadata().clone() })).collect()
}

/// Gives a cluster these sources of the quick search besides its pods, and the facts of
/// each store, as the core does.
pub(crate) fn with_sources(mut row: ClusterRow, more: Vec<Source>) -> ClusterRow {
    let data = row.data.as_mut().expect("the data of a connected cluster");
    let sources: Vec<Source> = [Source::full(data.pods.clone())].into_iter().chain(more).collect();
    data.stores = sources.iter().map(|source| source.status.clone()).collect();
    data.sources = sources.into();
    row
}

/// A live cluster with the pods of the scene and the Secrets in this state. A live store
/// has the Secret of the scene, and the Secrets page gets its full objects.
pub(crate) fn searchable(name: &str, secrets_state: StoreState) -> ClusterRow {
    let live_secrets = secrets_state.is_live();
    let snapshot: Snapshot<Meta> =
        if live_secrets { store(secrets_state, &metadata(&secrets())) } else { empty_store(secrets_state, now()) };
    let mut row = with_sources(live(name, &pods()), vec![Source::metadata(Kind::Secret, held(snapshot))]);
    if let Some(data) = row.data.as_mut().filter(|_| live_secrets) {
        data.objects = data.objects.clone().with(held(store(StoreState::Live, &secrets())));
    }
    row
}

/// Opens the quick search with its key, and types a text into its field.
pub(crate) fn search(window: &mut Window, text: &str) {
    window.key_press_modifiers(Modifiers::COMMAND, Key::K);
    window.run();
    let field = window.get_by_label("Search text");
    assert!(field.is_focused(), "the text field has the focus");
    field.type_text(text);
    window.run();
}

/// Waits for a hit with these words: `Secret kinds/web-secret of ams-prod`.
pub(crate) fn until_hit(window: &mut Window, hit: &str) {
    until(window, &format!("the hit {hit}"), |window| window.query_by_label(hit).is_some());
}
