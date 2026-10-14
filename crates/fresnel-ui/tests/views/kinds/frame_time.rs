//! The time of a frame of the table of a kind with 10,000 objects: a frame takes 16 ms or
//! less during scroll, sort and filter, as the Pods table (plan section 15). The worker
//! computes the value of each column of each row, so a frame reads no object. Objects of
//! extreme sizes keep the same limit: a frame builds only the texts on the screen.
//!
//! The limit holds in a release build:
//! `cargo test --release --package fresnel-ui --test views frame_time -- --nocapture`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use egui::{Event, Key};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ClusterRow;
use fresnel_core::store::StoreState;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment};
use k8s_openapi::api::core::v1::{ConfigMap, Pod};

use super::in_panel;
use super::scene::{deployments, live};
use crate::data::{data, held, pod, renamed, scene, store};
use crate::fake::{connected, live_status, open_page, shows, until, view, window_with_no_gpu};
use crate::frame_time::{Step, first, title, wheel};

const OBJECTS: usize = 10_000;

/// Copies of the Deployments of the scene, each with a name and a UID of its own, in 20
/// namespaces.
fn many_deployments() -> Vec<Arc<Deployment>> {
    let scene = deployments();
    let copy = |number: usize| {
        let mut object = Deployment::clone(&scene[number % scene.len()]);
        let name = object.metadata.name.take().unwrap_or_default();
        object.metadata.name = Some(format!("{name}-{number:05}"));
        object.metadata.uid = Some(format!("uid-{number:05}"));
        object.metadata.namespace = Some(format!("team-{:02}", number / scene.len() % 20));
        Arc::new(object)
    };
    (0..OBJECTS).map(copy).collect()
}

#[test]
fn a_frame_with_10_000_deployments_takes_16_ms_or_less_during_scroll_sort_and_filter() {
    let mut window = window_with_no_gpu(view(vec![live("ams-prod", &many_deployments())]), 2.0);
    open_page(&mut window, "Deployments");
    until(&mut window, "the rows", |window| shows(window, "10,000 Deployments"));
    let mut warm = Step::new("kinds: the first frames");
    (0..20).for_each(|_| warm.frame(&mut window));

    let mut scroll = Step::new("kinds: scroll");
    let rows = title(&window, "Namespace") + egui::vec2(0.0, 200.0);
    scroll.events(&mut window, [Event::PointerMoved(rows)]);
    let top = first(&window, "Name");
    scroll.events(&mut window, (0..300).map(|_| wheel(-60.0)));
    assert_ne!(first(&window, "Name"), top, "the table did scroll");
    scroll.events(&mut window, (0..300).map(|_| wheel(60.0)));
    scroll.until(&mut window, "the first row again", |window| first(window, "Name") == top);
    scroll.check();

    let mut sort = Step::new("kinds: sort");
    for (column, clicks, expected) in [("Ready", 2, "2/2"), ("Images", 2, "registry.invalid/fresnel/no-such-image:1")] {
        for click in 1..=clicks {
            let pos = title(&window, column);
            sort.click(&mut window, pos);
            if click < clicks {
                (0..30).for_each(|_| sort.frame(&mut window));
            }
        }
        let asked = Instant::now();
        sort.until(&mut window, &format!("the rows by {column}"), |window| {
            first(window, column).as_deref() == Some(expected)
        });
        eprintln!("frame time, kinds: the rows by {column} show after {:.1?}", asked.elapsed());
    }
    sort.check();

    let mut filter = Step::new("kinds: filter");
    let field = window.get_by_label("Filter Deployments").rect().center();
    filter.click(&mut window, field);
    for character in "broken".chars() {
        filter.events(&mut window, [Event::Text(character.to_string())]);
        (0..10).for_each(|_| filter.frame(&mut window));
    }
    filter.until(&mut window, "the rows of the filter", |window| shows(window, "2,500 of 10,000 Deployments"));
    filter.check();
}

/// A ConfigMap of almost 1 MiB: 70,000 keys with a value of one character.
fn config_map(number: usize) -> Arc<ConfigMap> {
    let mut object = ConfigMap::default();
    object.metadata.name = Some(format!("config-{number}"));
    object.metadata.namespace = Some("scene".to_owned());
    object.metadata.uid = Some(format!("uid-config-{number}"));
    object.data = Some((0..70_000).map(|key| (format!("key-{key:06}"), "v".to_owned())).collect::<BTreeMap<_, _>>());
    Arc::new(object)
}

/// Extreme sizes: the Keys column of a ConfigMap lists each key, and the panel of an object
/// reads its conditions from its JSON. The arrow keys open a new panel in each frame.
#[test]
fn a_frame_with_config_maps_of_70_000_keys_takes_16_ms_or_less_for_the_rows_and_the_arrow_keys() {
    let all: Vec<Arc<ConfigMap>> = (0..8).map(config_map).collect();
    let mut window = window_with_no_gpu(view(vec![live("ams-prod", &all)]), 2.0);
    open_page(&mut window, "ConfigMaps");
    until(&mut window, "the rows", |window| shows(window, "8 ConfigMaps"));

    let mut keys = Step::new("kinds: arrow keys on ConfigMaps of 70,000 keys");
    let row = window.get_by_label("config-0").rect().center();
    keys.click(&mut window, row);
    // The frame after the click gives the table the arrows: a real window draws it at once.
    keys.frame(&mut window);
    for _ in 1..8 {
        window.key_press(Key::ArrowDown);
        keys.frame(&mut window);
        keys.frame(&mut window);
    }
    assert!(in_panel(&window, "config-7"), "the panel follows the cursor to the last row");
    keys.check();

    // Each frame lays out the Keys of each row on the screen.
    let mut rows = Step::new("kinds: the rows of ConfigMaps of 70,000 keys");
    (0..60).for_each(|_| rows.frame(&mut window));
    let keys = first(&window, "Keys").expect("the keys of the first row");
    assert!(keys.starts_with("key-000000, key-000001") && keys.ends_with('…') && keys.chars().count() < 500);
    rows.check();
}

/// Extreme sizes: the panel of a DaemonSet on 5,000 nodes lists its pods from the owner
/// index in each frame.
#[test]
fn a_frame_with_the_panel_of_a_daemon_set_with_5_000_pods_takes_16_ms_or_less() {
    let agent = pod("agent-8djvd");
    let pods: Vec<Arc<Pod>> =
        (0..5000).map(|number| renamed(&agent, &format!("agent-{number:04}"), &format!("uid-{number:04}"))).collect();
    let mut cluster = data(store(StoreState::Live, &pods));
    cluster.objects = cluster.objects.with(held(store(StoreState::Live, &scene::<DaemonSet>())));
    let row = ClusterRow { data: Some(cluster), ..connected("ams-prod", live_status()) };
    let mut window = window_with_no_gpu(view(vec![row]), 2.0);
    open_page(&mut window, "DaemonSets");
    until(&mut window, "the rows", |window| shows(window, "1 DaemonSet"));

    let mut panel = Step::new("kinds: the panel of a DaemonSet with 5,000 pods");
    let row = window.get_by_label("agent").rect().center();
    panel.click(&mut window, row);
    (0..30).for_each(|_| panel.frame(&mut window));
    assert!(in_panel(&window, "agent-0000") && shows(&window, "and 4,900 more"), "the first pods and the count");
    panel.check();
}
