//! The Secrets and ConfigMaps of the key view tests, with made-up values only, and the view
//! of the core for the panel of one of them.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{ClusterRow, KeysView, ReadState, SaveJob};
use fresnel_core::cluster::ClusterId;
use fresnel_core::keys::{Entry, KeyKind, KeyObject, Place, SecretType, Target, Value, WriteState};
use fresnel_core::store::{ObjectKey, StoreState};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use crate::data::store;
use crate::fake::{Window, open_page, shows, until};
use crate::kinds::in_panel;
use crate::kinds::scene::with;

/// The namespace of the objects: the namespace of the pods of the scene, so the reference
/// index has the pods that use `web-secret` and `web-config`.
pub(crate) const NAMESPACE: &str = "scene";

/// A value of the read of a key view: a map, and bytes.
pub(crate) type Values<'a> = &'a [(&'a str, Place, &'a [u8])];

fn meta(name: &str, uid: &str) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.to_owned()),
        namespace: Some(NAMESPACE.to_owned()),
        uid: Some(uid.to_owned()),
        resource_version: Some("100".to_owned()),
        ..ObjectMeta::default()
    }
}

/// A Secret as its store holds it: its type and its keys with no value.
pub(crate) fn secret(name: &str, secret_type: &str, keys: &[&str]) -> Arc<Secret> {
    let data = keys.iter().map(|key| ((*key).to_owned(), ByteString(Vec::new()))).collect();
    let type_ = Some(secret_type.to_owned());
    Arc::new(Secret { metadata: meta(name, &format!("uid-{name}")), type_, data: Some(data), ..Secret::default() })
}

/// A ConfigMap as its store holds it: its keys with no value.
pub(crate) fn config_map(name: &str, keys: &[&str], binary: &[&str]) -> Arc<ConfigMap> {
    let data: BTreeMap<String, String> = keys.iter().map(|key| ((*key).to_owned(), String::new())).collect();
    let binary_data = binary.iter().map(|key| ((*key).to_owned(), ByteString(Vec::new()))).collect();
    let metadata = meta(name, &format!("uid-{name}"));
    Arc::new(ConfigMap { metadata, data: Some(data), binary_data: Some(binary_data), ..ConfigMap::default() })
}

/// The object of the key view of `name` in `cluster`, with the UID of the store.
pub(crate) fn target(cluster: &str, kind: KeyKind, name: &str) -> Target {
    let (key, uid) = (ObjectKey::new(NAMESPACE, name), format!("uid-{name}").into());
    Target { cluster: ClusterId::new(cluster), kind, key, uid }
}

/// The read of the server of an object with these values and this version.
pub(crate) fn read(target: &Target, version: &str, values: Values<'_>) -> Arc<KeyObject> {
    let entries = values
        .iter()
        .map(|(key, place, bytes)| ((*key).to_owned(), Entry { place: *place, value: Value::new(bytes.to_vec()) }));
    let secret_type = match target.kind {
        KeyKind::Secret => Some(SecretType::Opaque),
        KeyKind::ConfigMap => None,
    };
    Arc::new(KeyObject {
        target: target.clone(),
        version: version.into(),
        immutable: false,
        secret_type,
        entries: entries.collect(),
    })
}

/// The view of the core after a read of this object.
pub(crate) fn loaded(object: Arc<KeyObject>) -> KeysView {
    KeysView { target: object.target.clone(), object: Some(object), read: ReadState::Read, save: None }
}

/// The view of the core with a save of the panel in this state.
pub(crate) fn saving(view: &KeysView, sent: &fresnel_core::keys::Save, state: WriteState) -> KeysView {
    KeysView { save: Some(SaveJob { number: 1, save: sent.clone(), state }), ..view.clone() }
}

/// A live cluster with the pods of the scene and this store of Secrets.
pub(crate) fn with_secrets(name: &str, secrets: &[Arc<Secret>]) -> ClusterRow {
    with(name, store(StoreState::Live, secrets))
}

/// A live cluster with the pods of the scene and this store of ConfigMaps.
pub(crate) fn with_config_maps(name: &str, maps: &[Arc<ConfigMap>]) -> ClusterRow {
    with(name, store(StoreState::Live, maps))
}

/// Gives the cluster `cluster` of the fake core this view of the key view.
pub(crate) fn give(window: &mut Window, cluster: &str, keys: Option<KeysView>) {
    let row = window.state_mut().view.clusters.iter_mut().find(|row| row.id.as_str() == cluster);
    let data = row.and_then(|row| row.data.as_mut()).expect("a connected cluster");
    data.keys = keys.map(Arc::new);
    window.run();
}

/// Opens a page and the panel of the object of the row with this name.
pub(crate) fn open(window: &mut Window, page: &str, name: &str) {
    open_page(window, page);
    until(window, "the rows", |window| shows(window, name));
    window.get_by_label(name).click();
    window.run();
    until(window, "the panel", |window| in_panel(window, "DATA"));
}

/// True when the frame shows a text that has `part` in it.
pub(crate) fn shows_part(window: &Window, part: &str) -> bool {
    window.query_all_by_label_contains(part).next().is_some()
}
