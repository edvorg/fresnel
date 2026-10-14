//! The tab Manifest of the detail panel (plan section 12.9): the read-only YAML of the
//! object, which the core reads from the server for each version of the object.

use std::sync::Arc;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{Command, ManifestRead, ManifestRequest, ManifestState};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::{ManagedFields, Manifest};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::apps::v1::Deployment;

use super::scene::{deployments, live, uid_of};
use super::weakness::page;
use crate::data::changed_object;
use crate::fake::{Window, error, shows, view};

/// The request of the tab for the Deployment `web`, with its UID.
fn request(managed_fields: ManagedFields) -> ManifestRequest {
    let (cluster, key) = (ClusterId::new("ams-prod"), ObjectKey::new("scene", "web"));
    ManifestRequest { cluster, kind: Kind::Deployment, key, uid: uid_of(&deployments(), "web"), managed_fields }
}

/// The count of the reads that the tab asked for.
fn reads(window: &Window) -> usize {
    window.state().sent.iter().filter(|command| matches!(command, Command::ReadManifest(_))).count()
}

/// The core gives this state of the read of the request.
fn answer(window: &mut Window, request: ManifestRequest, state: ManifestState) {
    window.state_mut().view.manifest = Some(Arc::new(ManifestRead { request, number: 1, state }));
    window.run();
}

#[test]
fn the_tab_asks_for_each_version_of_its_object_and_shows_the_lines_on_the_screen() {
    let all = deployments();
    let mut window = page::<Deployment>(vec![live("ams-prod", &all)], 4);
    window.get_by_label("web").click();
    window.run();
    window.get_by_label("Manifest").click();
    window.run();
    let removed = request(ManagedFields::Removed);
    assert_eq!(window.state().sent.last(), Some(&Command::ReadManifest(removed.clone())));
    assert!(shows(&window, "The manifest is read from ams-prod…"));

    // A manifest of 10,003 lines: the tab lays out the lines on the screen only.
    let lines: Vec<String> = (0..10_000).map(|number| format!("  line-{number:05}: x")).collect();
    let text = format!("kind: Deployment\nmetadata:\n  name: web\n{}\n", lines.join("\n"));
    let manifest = Manifest { text, version: Some("42".to_owned()), managed_fields: ManagedFields::Removed };
    answer(&mut window, removed.clone(), ManifestState::Done(Arc::new(manifest)));
    assert!(shows(&window, "kind: Deployment") && shows(&window, "  line-00010: x") && shows(&window, "version 42"));
    assert!(!shows(&window, "  line-09999: x"), "a line below the screen has no layout");
    assert_eq!(reads(&window), 1, "one read for one version");

    // The switch of managedFields asks for the other form, and a failed read tells why.
    window.get_by_label("managedFields").click();
    window.run();
    let kept = request(ManagedFields::Kept);
    assert_eq!(window.state().sent.last(), Some(&Command::ReadManifest(kept.clone())));
    answer(&mut window, kept.clone(), ManifestState::Failed(error("the request got no answer in 10s")));
    assert!(shows(&window, "The read failed: the request got no answer in 10s"));
    window.get_by_label("Read the manifest again").click();
    window.run();
    assert_eq!(reads(&window), 3);

    // A new version of the object in its store asks again, after the end of the read in
    // progress: an object that changes faster than a read would else never show.
    let web = all.iter().find(|object| object.metadata.name.as_deref() == Some("web")).expect("web");
    let others = all.iter().filter(|object| !Arc::ptr_eq(object, web)).cloned();
    let version = |number: u32| {
        let next = changed_object(&**web, |value| value["metadata"]["resourceVersion"] = number.to_string().into());
        view(vec![live("ams-prod", &others.clone().chain([next]).collect::<Vec<_>>())])
    };
    for number in 43..46 {
        window.state_mut().view = version(number);
        answer(&mut window, kept.clone(), ManifestState::Reading);
    }
    assert_eq!(reads(&window), 3, "no read during the read in progress");
    window.state_mut().view = version(46);
    answer(&mut window, kept.clone(), ManifestState::Failed(error("the request got no answer in 10s")));
    assert_eq!(reads(&window), 4, "the newest version after the end of the read");

    // A deleted object asks for no read: its server has no such object.
    window.state_mut().view = view(vec![live("ams-prod", &others.clone().collect::<Vec<_>>())]);
    window.run();
    assert!(shows(&window, "The object left its store. The tab shows the last manifest that it read."));
    assert_eq!(reads(&window), 4);

    // A new Deployment gets the name: its panel asks with its own UID, so the read of the
    // deleted one (which the core refuses for the new UID) never shows in its panel.
    let new = changed_object(&**web, |value| value["metadata"]["uid"] = "the-uid-of-a-new-web".into());
    window.state_mut().view = view(vec![live("ams-prod", &others.chain([new]).collect::<Vec<_>>())]);
    window.get_by_label("Close the details").click();
    window.run();
    window.get_by_label("web").click();
    window.run();
    window.get_by_label("Manifest").click();
    window.run();
    let asked = ManifestRequest { uid: "the-uid-of-a-new-web".into(), ..request(ManagedFields::Removed) };
    assert_eq!(window.state().sent.last(), Some(&Command::ReadManifest(asked)));
    assert!(!shows(&window, "kind: Deployment"), "the text of the read for the other UID");
}
