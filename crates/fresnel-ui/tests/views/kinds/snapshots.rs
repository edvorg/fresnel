//! PNG files of the pages of the kinds in the dark theme, for a look by a person.

use std::sync::Arc;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{ClusterRow, ManifestRead, ManifestRequest, ManifestState};
use fresnel_core::cluster::{ClusterId, LinkState};
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::{ManagedFields, Manifest};
use fresnel_core::store::{ObjectKey, StoreState};
use k8s_openapi::api::apps::v1::ReplicaSet;

use super::scene::{config_maps, deployments, live, nodes, services, uid_of, with};
use crate::data::{empty_store, held, scene, store};
use crate::fake::{SECOND, Window, error, now, open_page, row, until, view, window_with};
use crate::pods::scenes::{forbidden, no_answer};
use crate::snapshots::{SCALE, save_clean};

fn open(window: &mut Window, page: &str, name: &str) {
    open_page(window, page);
    until(window, "the rows", |window| window.query_all_by_label(name).next().is_some());
    window.query_all_by_label(name).next().expect("a row").click();
    window.run();
}

#[test]
fn the_deployments_of_two_clusters_with_the_panel_of_one() {
    let mut data = live("ams-prod", &deployments()).data.expect("data");
    data.objects = data.objects.with(held(store(StoreState::Live, &scene::<ReplicaSet>())));
    let ams = ClusterRow { data: Some(data), ..live("ams-prod", &deployments()) };
    let mut window = window_with(view(vec![ams, live("fra-stage", &deployments())]), egui::Theme::Dark, SCALE);
    open(&mut window, "Deployments", "broken");
    save_clean(&mut window, "kinds-deployments");
}

#[test]
fn the_config_maps_with_the_notes_of_three_clusters_that_fail() {
    let pause = StoreState::Backoff { failures: 3, reason: no_answer(), until: now() + 2 * SECOND };
    let clusters = vec![
        live("ams-prod", &config_maps()),
        with("sgp-prod", store(pause, &config_maps())),
        with::<k8s_openapi::api::core::v1::ConfigMap>(
            "nyc-prod",
            empty_store(StoreState::Forbidden { reason: forbidden() }, now() - 20 * SECOND),
        ),
        row("lab", LinkState::Failed(error("the cluster entry `lab` has no certificate authority"))),
    ];
    let mut window = window_with(view(clusters), egui::Theme::Dark, SCALE);
    open(&mut window, "ConfigMaps", "web-config");
    save_clean(&mut window, "kinds-config-maps");
}

#[test]
fn the_node_with_its_conditions_and_its_pods() {
    let mut window = window_with(view(vec![live("ams-prod", &nodes())]), egui::Theme::Dark, SCALE);
    let name = nodes()[0].metadata.name.clone().expect("a name");
    open(&mut window, "Nodes", &name);
    save_clean(&mut window, "kinds-node");
}

#[test]
fn the_manifest_tab_of_a_service() {
    let mut window = window_with(view(vec![live("ams-prod", &services())]), egui::Theme::Dark, SCALE);
    open(&mut window, "Services", "web");
    window.get_by_label("Manifest").click();
    window.run();
    let service = services().into_iter().find(|service| service.metadata.name.as_deref() == Some("web"));
    let mut object = serde_json::to_value(&*service.expect("web")).expect("JSON");
    object["apiVersion"] = "v1".into();
    object["kind"] = "Service".into();
    let text = serde_saphyr::to_string(&object).expect("YAML");
    let request = ManifestRequest {
        cluster: ClusterId::new("ams-prod"),
        kind: Kind::Service,
        key: ObjectKey::new("scene", "web"),
        uid: uid_of(&services(), "web"),
        managed_fields: ManagedFields::Removed,
    };
    let manifest = Manifest { text, version: Some("1042".to_owned()), managed_fields: ManagedFields::Removed };
    let state = ManifestState::Done(Arc::new(manifest));
    window.state_mut().view.manifest = Some(Arc::new(ManifestRead { request, number: 1, state }));
    window.run();
    save_clean(&mut window, "kinds-manifest");
}
