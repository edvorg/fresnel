//! The time of a frame of the table of a kind with 10,000 objects: a frame takes 16 ms or
//! less during scroll, sort and filter, as the Pods table (plan section 15). The worker
//! computes the value of each column of each row, so a frame reads no object.
//!
//! The limit holds in a release build:
//! `cargo test --release --package fresnel-ui --test views frame_time -- --nocapture`.

use std::sync::Arc;
use std::time::Instant;

use egui::Event;
use egui_kittest::kittest::Queryable as _;
use k8s_openapi::api::apps::v1::Deployment;

use super::scene::{deployments, live};
use crate::fake::{open_page, shows, until, view, window_with_no_gpu};
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
    for (column, clicks, expected) in [("Ready", 1, "0/1"), ("Images", 2, "registry.invalid/fresnel/no-such-image:1")] {
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
