//! The drags of the edges of the window with the mouse: the top of the dock, the left of the
//! panel of an object and the left of the chosen part of a tab. A test or a picture that needs
//! more room drags an edge, as the user does, and asserts that the edge moved.

use egui_kittest::kittest::Queryable as _;

use crate::fake::Window;

/// A drag with the mouse from one place to an other, as on the edge of a panel: the
/// pointer takes the edge in one frame and lets it go in a later one.
pub(crate) fn move_edge(window: &mut Window, from: egui::Pos2, to: egui::Pos2) {
    window.hover_at(from);
    window.run();
    window.drag_at(from);
    window.run();
    window.hover_at(to);
    window.run();
    window.drop_at(to);
    window.run();
}

/// Asserts that a drag of an edge by `by` points moved `what` by `moved` points. The press
/// takes an edge up to the grab radius away from it, and the edge then jumps to the
/// pointer; a press that missed the edge moves nothing.
pub(crate) fn assert_dragged(window: &Window, what: &str, moved: f32, by: f32) {
    let radius = window.ctx.global_style().interaction.resize_grab_radius_side;
    assert!((moved - by).abs() <= radius, "{what} moved by {moved} points, not by {by}");
}

/// The top edge of the dock is this far above its tabs: a press there takes the edge.
const DOCK_EDGE_ABOVE_TABS: f32 = 4.0;
/// The press on the top edge of the dock is this far from the right end of the window,
/// where the tab bar of the dock has no tab.
const DOCK_EDGE_FROM_RIGHT: f32 = 200.0;

/// Drags the top edge of the dock `by` points up: the dock is then that much taller, and
/// its tabs moved up with the edge.
pub(crate) fn grow_dock(window: &mut Window, by: f32) {
    let before = dock_edge(window);
    let edge = egui::pos2(window.ctx.content_rect().right() - DOCK_EDGE_FROM_RIGHT, before);
    move_edge(window, edge, edge - egui::vec2(0.0, by));
    assert_dragged(window, "the top of the dock", before - dock_edge(window), by);
}

/// The top edge of the dock, from its tabs.
pub(crate) fn dock_edge(window: &Window) -> f32 {
    let tabs = window.get_by_role_and_label(egui::accesskit::Role::Button, "Operations").rect();
    tabs.top() - DOCK_EDGE_ABOVE_TABS
}

/// The left edge of the panel of an object is this far left of its tabs.
const PANEL_EDGE_LEFT_OF_TABS: f32 = 12.0;
/// The press on that edge is this far below the tabs, beside the fields of the panel.
const PANEL_EDGE_BELOW_TABS: f32 = 100.0;

/// Drags the left edge of the panel of an object `by` points to the left: the panel is
/// then that much wider, and its tabs moved left with the edge. It is 720 points wide at
/// most.
pub(crate) fn widen_panel(window: &mut Window, by: f32) {
    let tabs = |window: &Window| window.get_by_role_and_label(egui::accesskit::Role::Button, "Overview").rect();
    let before = tabs(window);
    let edge = egui::pos2(before.left() - PANEL_EDGE_LEFT_OF_TABS, before.bottom() + PANEL_EDGE_BELOW_TABS);
    move_edge(window, edge, edge - egui::vec2(by, 0.0));
    assert_dragged(window, "the left of the panel", before.left() - tabs(window).left(), by);
}

/// The part of the chosen job of the Operations tab and the part of the chosen forward of the
/// Forwards tab have a margin of this many points at their left, so the left edge of each is
/// that far left of its text.
const CHOSEN_MARGIN: f32 = 8.0;
/// The part of the chosen job grows this much in the test of a drain and in its picture: the
/// verdict "keep: a pod of a DaemonSet" then fits whole.
pub(crate) const JOB_WIDENING: f32 = 200.0;

/// Drags the left edge of the chosen part of the Operations tab or of the Forwards tab `by`
/// points to the left; a negative `by` drags it to the right. `title` gives the left of the
/// title of the part, and the press is at the height `y`.
pub(crate) fn drag_chosen(window: &mut Window, title: impl Fn(&Window) -> f32, y: f32, by: f32) {
    let before = title(window);
    let edge = egui::pos2(before - CHOSEN_MARGIN, y);
    move_edge(window, edge, edge - egui::vec2(by, 0.0));
    assert_dragged(window, "the left of the chosen part", before - title(window), by);
}

/// Drags the left edge of the part of the chosen job of the Operations tab `by` points to the
/// left. `title` is a part of the title of the job, and the press is at the height of the
/// text `row` of the job: the rows of the job list end higher up.
pub(crate) fn widen_job(window: &mut Window, (title, row): (&str, &str), by: f32) {
    let y = window.get_by_label_contains(row).rect().center().y;
    drag_chosen(window, |window| window.get_by_label_contains(title).rect().left(), y, by);
}
