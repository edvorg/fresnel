//! Fast keys and new data in the quick search. The hits of a search arrive from a worker
//! thread some frames after the key, and the stores change at any moment, so a key must
//! act on the row that the screen shows.

use egui::accesskit::Toggled;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::ClusterRow;
use fresnel_core::store::{Snapshot, StoreState};

use super::{search, until_hit, with_sources};
use crate::data::{cluster, pod, pods, renamed, store};
use crate::fake::{Window, in_one_frame, press, until, view, window};
use crate::kinds::in_panel;

/// The cluster `ams-prod` with the pods of the scene, these more pods, and the number of its
/// store: a new number is a new snapshot.
fn with_pods(more: &[&str], generation: u64) -> ClusterRow {
    let mut all = pods();
    all.extend(more.iter().map(|name| renamed(&pod("crash"), name, &format!("uid-{name}"))));
    let snapshot = Snapshot { generation, ..store(StoreState::Live, &all) };
    with_sources(cluster("ams-prod", snapshot), Vec::new())
}

/// True when the row with these words has the cursor.
fn has_cursor(window: &Window, row: &str) -> bool {
    window.query_by_label(row).is_some_and(|node| node.accesskit_node().toggled() == Some(Toggled::True))
}

/// Presses Enter and waits for the panel of the object that it opened.
fn enter_and_wait_for_a_panel(window: &mut Window) {
    window.key_press(Key::Enter);
    until(window, "a panel", |window| window.query_by_label("Overview").is_some());
}

/// Plan section 11.2: the cursor holds the identity of its object. New hits before it do
/// not move it to an other object, so Enter opens the object that it showed.
#[test]
fn the_cursor_stays_on_its_hit_when_new_hits_come_before_it() {
    let mut window = window(view(vec![with_pods(&[], 1)]));
    search(&mut window, "crash");
    let hit = "Pod scene/init-crash of ams-prod";
    until_hit(&mut window, hit);
    window.key_press(Key::ArrowDown);
    window.run();
    assert!(has_cursor(&window, hit));

    window.state_mut().view = view(vec![with_pods(&["crash-b"], 2)]);
    until_hit(&mut window, "Pod scene/crash-b of ams-prod");
    assert!(has_cursor(&window, hit), "the cursor stays on its object");
    enter_and_wait_for_a_panel(&mut window);
    assert!(in_panel(&window, "init-crash"));
}

/// A slow frame gets each key that the user pressed since the frame before. Each press of
/// an arrow moves the cursor one row.
#[test]
fn each_press_of_an_arrow_in_one_frame_moves_the_cursor_one_row() {
    let mut window = window(view(vec![with_pods(&[], 1)]));
    search(&mut window, "init");
    until_hit(&mut window, "Pod scene/init-wait of ams-prod");
    in_one_frame(
        &mut window,
        [press(Modifiers::NONE, Key::ArrowDown), press(Modifiers::NONE, Key::ArrowDown)].concat(),
    );
    window.run();
    assert!(has_cursor(&window, "Pod scene/init-wait of ams-prod"), "two rows down");
    let up = press(Modifiers::NONE, Key::ArrowUp);
    in_one_frame(&mut window, [up.clone(), up.clone(), up].concat());
    window.run();
    assert!(has_cursor(&window, "Pod scene/init-crash of ams-prod"), "the first row stops the cursor");
}

/// Enter right after the last letter comes before the hits of the new text. It opens the
/// best hit of that text, not a hit of the text before it.
#[test]
fn enter_before_the_hits_of_the_text_opens_the_best_hit_of_that_text() {
    let mut window = window(view(vec![with_pods(&[], 1)]));
    search(&mut window, "init-");
    until_hit(&mut window, "Pod scene/init-crash of ams-prod");
    let last_letter = egui::Event::Text("w".to_owned());
    in_one_frame(&mut window, [vec![last_letter], press(Modifiers::NONE, Key::Enter).to_vec()].concat());
    until(&mut window, "a panel", |window| window.query_by_label("Overview").is_some());
    assert!(in_panel(&window, "init-wait"), "the hit of init-w");
}

/// Design rule 12: the text of the search has a limit, so a paste of a large text gives
/// the worker a bounded job.
#[test]
fn a_paste_of_a_large_text_keeps_the_limit_of_the_text() {
    let mut window = window(view(vec![with_pods(&[], 1)]));
    search(&mut window, &"crash ".repeat(2_000));
    let text = window.get_by_label("Search text").value().unwrap_or_default();
    assert!(!text.is_empty() && text.chars().count() <= 512, "{} characters", text.chars().count());
    until_hit(&mut window, "Pod scene/crash of ams-prod");
}
