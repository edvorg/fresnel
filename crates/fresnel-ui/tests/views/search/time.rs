//! The time of the quick search with 4 clusters (plan section 15, milestone M2): the hits
//! show 50 ms or less after the key that completes the text.
//!
//! Each cluster has 10,000 made-up pods and the metadata of 5,000 made-up Secrets: 60,000
//! objects, more than the 59,000 rows of the store prototype. The time runs from the key to
//! the frame that shows the hits, so it has the frames, the request, the worker thread and
//! its wake of the window. The limit holds in a release build on the Mac of the 16 ms frame:
//! `cargo test --release --package fresnel-ui --test views search::time -- --nocapture`.
//! A debug build runs the same steps and checks no time. CI gives the limit of its runner.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::kinds::Kind;
use fresnel_core::search::Source;
use fresnel_core::store::StoreState;
use k8s_openapi::api::core::v1::Secret;

use super::{metadata, secrets, with_sources};
use crate::data::{changed_object, held, live, many_pods, store};
use crate::fake::{Window, view, window_with_no_gpu};
use crate::frame_time::limit;

const LIMIT: Duration = Duration::from_millis(50);
/// The variable with the limit of a slower machine, in milliseconds.
const LIMIT_VARIABLE: &str = "FRESNEL_SEARCH_LIMIT_MS";
const CLUSTERS: usize = 4;
const PODS: usize = 10_000;
const SECRETS: usize = 5_000;
/// The longest wait for the hits in a debug build.
const WAIT: Duration = Duration::from_secs(60);

/// Made-up Secrets: copies of the Secret of the scene, each with a name of its own.
fn many_secrets() -> Vec<Arc<Secret>> {
    let model = secrets().into_iter().next().expect("the Secret of the scene");
    let copy = |number: usize| {
        changed_object(&*model, |value| {
            value["metadata"]["name"] = format!("token-{number:05}").into();
            value["metadata"]["uid"] = format!("secret-{number:05}").into();
        })
    };
    (0..SECRETS).map(copy).collect()
}

fn clusters() -> Vec<fresnel_core::app::ClusterRow> {
    let (pods, secrets) = (many_pods(PODS), metadata(&many_secrets()));
    let cluster = |number: usize| {
        let secrets = Source::metadata(Kind::Secret, held(store(StoreState::Live, &secrets)));
        with_sources(live(&format!("cluster-{number}"), &pods), vec![secrets])
    };
    (1..=CLUSTERS).map(cluster).collect()
}

/// Types the last key of a text and draws frames with no pause until `shown`. Gives the time.
fn time_to(window: &mut Window, last: char, shown: impl Fn(&Window) -> bool) -> Duration {
    let started = Instant::now();
    window.get_by_label("Search text").type_text(&last.to_string());
    while !shown(window) {
        assert!(started.elapsed() < WAIT, "the hits show in time");
        window.step();
    }
    started.elapsed()
}

#[test]
fn the_hits_of_4_clusters_with_60000_objects_show_in_the_search_limit() {
    let mut window = window_with_no_gpu(view(clusters()), 1.0);
    window.key_press_modifiers(Modifiers::COMMAND, Key::K);
    window.run();
    // One name, and a word that most names have: its hits need the choice of the best ones.
    let pod = many_pods(PODS).pop().expect("a pod");
    let (name, namespace) = (pod.metadata.name.clone().expect("a name"), pod.metadata.namespace.clone().expect("one"));
    let (start, last) = name.split_at(name.len() - 1);
    window.get_by_label("Search text").type_text(start);
    window.run();
    let hit = format!("Pod {namespace}/{name} of cluster-1");
    let one = time_to(&mut window, last.chars().next().expect("a last letter"), |window| {
        window.query_by_label(&hit).is_some()
    });

    window.key_press_modifiers(Modifiers::COMMAND, Key::A);
    window.key_press(Key::Backspace);
    window.get_by_label("Search text").type_text("c");
    window.run();
    let broad = time_to(&mut window, 'r', |window| window.query_by_label_contains("200 of ").is_some());
    println!("quick search with {} objects: one name {one:?}, a broad word {broad:?}", CLUSTERS * (PODS + SECRETS));
    if cfg!(debug_assertions) {
        return;
    }
    let limit = limit(LIMIT_VARIABLE, LIMIT);
    assert!(one <= limit && broad <= limit, "{one:?} and {broad:?}, the limit is {limit:?}");
}
