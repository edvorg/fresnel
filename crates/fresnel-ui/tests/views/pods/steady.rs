//! The rows stay under the pointer when a note above the table comes or goes away. egui gives
//! a press to the row that the frame before the press had at the pointer, and a person aims
//! at the rows of a frame some time before the press. The note of a store that waits for the
//! start of its watch goes away when the watch opens, and comes when the watch ends: with no
//! hold, the rows moved by the height of the note, and the click opened the row below or above
//! the row of the person.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::epaint::ClippedShape;
use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, Shape, TouchPhase};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::CoreView;
use fresnel_core::store::{StoreState, Wait};
use k8s_openapi::api::core::v1::Pod;

use crate::data::{cluster, live, many_pods, pods, store};
use crate::fake::{Window, shows, view, window};

const NOTE: &str = "Pods: waits for the start of its watch. Its rows are old since 07:59:12.";

/// The first row of a pod: the label with its name nearest to the left, not the heading of
/// the panel, and of those the one nearest to the top.
fn row(window: &Window, name: &str) -> Rect {
    let labels = window.query_all_by_label(name).map(|node| node.rect());
    labels.min_by(|a, b| a.left().total_cmp(&b.left()).then(a.top().total_cmp(&b.top()))).expect("the row")
}

fn button(place: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: place, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// A click of a person at this place: the move, the press and the release, each in a frame
/// of its own, as a window gets them and as the harness gives them.
fn click_at(window: &mut Window, place: Pos2) {
    for event in [Event::PointerMoved(place), button(place, true), button(place, false)] {
        window.event(event);
    }
    window.run();
}

/// A place in the sidebar at the height of this place: the pointer leaves the table with no
/// move up or down, which a press would give to the scroll of the rows.
fn beside_the_table(window: &Window, place: Pos2) -> Pos2 {
    let sidebar = window.get_by_role_and_label(Role::RadioButton, "Pods").rect();
    Pos2::new(sidebar.center().x, place.y)
}

/// The count of shapes that the frame paints after the last shape of this text at the height of
/// a label of it, from the left to the right side of the window. A table that keeps its top
/// under a note that came paints its scroll bar and the fades of its scroll area there, and
/// then paints the parts above it again over them.
fn painted_over(window: &Window, text: &str) -> usize {
    let width = window.ctx.content_rect().x_range();
    let lines: Vec<Rect> =
        window.query_all_by_label(text).map(|node| Rect::from_x_y_ranges(width, node.rect().y_range())).collect();
    let shapes = &window.output().shapes;
    let is_text = |clipped: &ClippedShape| matches!(&clipped.shape, Shape::Text(shape) if shape.galley.text() == text);
    let painted = shapes.iter().rposition(is_text).expect("the frame paints the text");
    let covers = |clipped: &&ClippedShape| {
        let shown = clipped.clip_rect.intersect(clipped.shape.visual_bounding_rect());
        lines.iter().any(|line| shown.intersect(line.shrink2(egui::vec2(0.0, 1.0))).is_positive())
    };
    shapes.get(painted + 1..).unwrap_or_default().iter().filter(covers).count()
}

/// True when the frame paints, after the first shape of this text, an opaque fill over each
/// label of it to the scroll bar at the right side of the page: what the frame painted before
/// that fill does not show there.
fn filled_over(window: &Window, text: &str) -> bool {
    // The page ends 8 points before the right side of the window, and a scroll bar is 8 points wide.
    let bar = window.ctx.content_rect().right() - 12.0;
    let places: Vec<Rect> = window
        .query_all_by_label(text)
        .map(|node| node.rect().union(Rect::from_x_y_ranges(bar..=bar, node.rect().y_range())))
        .collect();
    let shapes = &window.output().shapes;
    let is_text = |clipped: &ClippedShape| matches!(&clipped.shape, Shape::Text(shape) if shape.galley.text() == text);
    let first = shapes.iter().position(is_text).expect("the frame paints the text");
    let fills = |clipped: &ClippedShape| match &clipped.shape {
        Shape::Rect(rect) if rect.fill.is_opaque() => {
            places.iter().all(|place| clipped.clip_rect.intersect(rect.rect).contains_rect(*place))
        }
        _ => false,
    };
    shapes.get(first + 1..).unwrap_or_default().iter().any(fills)
}

#[test]
fn a_note_that_goes_away_in_the_frame_of_a_click_moves_no_row_under_the_pointer() {
    let mut window = window(view(vec![cluster("ams-prod", store(StoreState::Waiting(Wait::Stream), &pods()))]));
    assert!(shows(&window, NOTE));
    let place = row(&window, "db-0").center();

    // The watch opens, so the frame of the move of the click has no note.
    window.state_mut().view = view(vec![live("ams-prod", &pods())]);
    click_at(&mut window, place);
    assert!(!shows(&window, NOTE), "the store is live");
    let pod_index = |number: u32| shows(&window, &format!("apps.kubernetes.io/pod-index={number}"));
    assert!(pod_index(0) && !pod_index(1), "the click opens db-0, the row at the pointer, and not db-1");
    assert!((row(&window, "db-0").center().y - place.y).abs() < 0.5, "the row stays under the pointer");

    // The pointer leaves the table, and the rows take the place of the note.
    window.event(Event::PointerMoved(beside_the_table(&window, place)));
    window.run();
    assert!(row(&window, "db-0").center().y < place.y - 10.0, "the rows move up");
}

/// Two clusters, so the two notes are higher than the titles and cover the top rows too.
#[test]
fn a_note_that_comes_in_the_frame_of_a_click_moves_no_row_under_the_pointer() {
    let two = |state: StoreState| {
        let pods = pods();
        view(vec![cluster("ams-prod", store(state.clone(), &pods)), cluster("fra-prod", store(state, &pods))])
    };
    let mut window = window(two(StoreState::Live));
    assert!(!shows(&window, NOTE));
    let place = row(&window, "db-1").center();

    // The watches end and wait for their start again, so the frame of the move of the click
    // has the notes. They show at once, over the top of the table.
    window.state_mut().view = two(StoreState::Waiting(Wait::Stream));
    click_at(&mut window, place);
    assert_eq!(window.query_all_by_label(NOTE).count(), 2, "the notes show at once");
    assert_eq!(painted_over(&window, NOTE), 0, "no part of the table paints over the notes");
    let pod_index = |number: u32| shows(&window, &format!("apps.kubernetes.io/pod-index={number}"));
    assert!(pod_index(1) && !pod_index(0), "the click opens db-1, the row at the pointer, and not db-0");
    assert!((row(&window, "db-1").center().y - place.y).abs() < 0.5, "the row stays under the pointer");

    // The pointer leaves the table, and the rows move below the note.
    window.event(Event::PointerMoved(beside_the_table(&window, place)));
    window.run();
    assert!(row(&window, "db-1").center().y > place.y + 10.0, "the rows move down");
    assert_eq!(painted_over(&window, NOTE), 0);
}

/// A press holds the place until its release, also when the pointer leaves the table first.
#[test]
fn the_rows_stay_until_the_release_of_a_press_on_the_table() {
    let mut window = window(view(vec![cluster("ams-prod", store(StoreState::Waiting(Wait::Stream), &pods()))]));
    let place = row(&window, "db-0").center();
    for event in [Event::PointerMoved(place), button(place, true)] {
        window.event(event);
    }
    window.run();

    // The note goes away while the button is down and the pointer is beside the table.
    window.state_mut().view = view(vec![live("ams-prod", &pods())]);
    let beside = beside_the_table(&window, place);
    window.event(Event::PointerMoved(beside));
    window.run();
    assert!(!shows(&window, NOTE), "the store is live");
    assert!((row(&window, "db-0").center().y - place.y).abs() < 0.5, "the row stays during the press");

    window.event(button(beside, false));
    window.run();
    assert!(row(&window, "db-0").center().y < place.y - 10.0, "the rows move up after the release");
}

/// Five clusters with these pods, whose stores are in this state. Five notes are more than
/// the place of the notes shows, so they scroll in it with a bar of their own.
fn five(state: &StoreState, pods: &[Arc<Pod>]) -> CoreView {
    let names = ["ams-prod", "fra-prod", "lon-prod", "nyc-prod", "sfo-prod"];
    view(names.iter().map(|name| cluster(name, store(state.clone(), pods))).collect())
}

/// The name in a row of the table that shows, from the top: a label of a pod in the leftmost
/// column.
fn name_in_row(window: &Window, number: usize) -> String {
    let names = window.query_all_by_label_contains("-000");
    let mut named: Vec<(Rect, String)> = names.map(|node| (node.rect(), node.value().unwrap_or_default())).collect();
    named.sort_by(|(a, _), (b, _)| a.left().total_cmp(&b.left()).then(a.top().total_cmp(&b.top())));
    named.into_iter().nth(number).expect("the row").1
}

/// The top of the first note, which a scroll of the notes moves.
fn top_of_notes(window: &Window) -> f32 {
    window.query_all_by_label(NOTE).map(|node| node.rect().top()).fold(f32::MAX, f32::min)
}

/// The notes come while the pointer is over the table, and the table keeps its top under them.
/// A pointer over the notes is over them, not over the table: the drag of their scroll bar
/// scrolls them, and the scroll bar of the table, which was below them, takes no press.
#[test]
fn a_press_on_the_notes_that_came_over_the_table_goes_to_the_notes() {
    let pods = many_pods(60);
    let mut window = window(five(&StoreState::Live, &pods));
    let (first, aimed) = (name_in_row(&window, 0), name_in_row(&window, 8));
    let place = row(&window, &aimed).center();
    window.event(Event::PointerMoved(place));
    window.run();
    window.state_mut().view = five(&StoreState::Waiting(Wait::Stream), &pods);
    window.run();
    let top = top_of_notes(&window);
    assert!((row(&window, &aimed).center().y - place.y).abs() < 0.5, "the table keeps its top");

    // The scroll bar of the notes is at the right side of their place, below their top: the page
    // ends 8 points before the right side of the window, and the bar is 8 points wide.
    let bar = Pos2::new(window.ctx.content_rect().right() - 12.0, top + 12.0);
    let below = Pos2::new(bar.x, bar.y + 40.0);
    for event in [Event::PointerMoved(bar), button(bar, true), Event::PointerMoved(below), button(below, false)] {
        window.event(event);
        window.run();
    }
    assert!(top_of_notes(&window) < top - 10.0, "the notes scroll");
    assert_eq!(name_in_row(&window, 0), first, "the table does not scroll");
}

/// A table that scrolled down fades the top of its rows, and many rows have a scroll bar. The
/// notes that come while the pointer is over the table show over its top with neither of them.
#[test]
fn a_note_that_comes_over_a_table_that_scrolled_shows_with_nothing_of_the_table_over_it() {
    let pods = many_pods(200);
    let two = |state: StoreState| {
        view(vec![cluster("ams-prod", store(state.clone(), &pods)), cluster("fra-prod", store(state, &pods))])
    };
    let mut window = window(two(StoreState::Live));
    let place = row(&window, &name_in_row(&window, 5)).center();
    let wheel = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -300.0),
        modifiers: Modifiers::NONE,
        phase: TouchPhase::Move,
    };
    for event in [Event::PointerMoved(place), wheel] {
        window.event(event);
    }
    window.run();
    let aimed = name_in_row(&window, 5);
    let at = row(&window, &aimed).center().y;

    window.state_mut().view = two(StoreState::Waiting(Wait::Stream));
    window.run();
    assert_eq!(window.query_all_by_label(NOTE).count(), 2, "the notes show at once");
    assert!((row(&window, &aimed).center().y - at).abs() < 0.5, "the table keeps its top");
    assert!(filled_over(&window, NOTE), "the scroll bar and the fades of the table do not show at the notes");
    assert_eq!(painted_over(&window, NOTE), 0, "no part of the table paints over the notes");
}
