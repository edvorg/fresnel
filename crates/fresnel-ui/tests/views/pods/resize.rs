//! The widths of the columns. A drag of the edge at the right of a title moves that edge
//! with the pointer, and no other column changes its width: in the first frames after the
//! table shows and later, from left to right and from right to left, and in a window that
//! is wider and one that is narrower than the sum of the columns, also at the right end of
//! a table that scrolls to the side. The widths stay through new data, a sort, a filter and
//! a page that hides the table.

use egui::accesskit::Role;
use egui::{Rangef, pos2};
use egui_kittest::kittest::Queryable as _;

use crate::data::{live, pod, pods};
use crate::fake::{Window, open_page, shows, until, view, window};

/// The titles of the columns of the Pod table, in their sequence.
pub(crate) const TITLES: [&str; 11] =
    ["Name", "Namespace", "Cluster", "Ready", "Status", "Restarts", "Last restart", "Node", "Age", "CPU", "Memory"];
/// The widths that a column can have.
const WIDTHS: Rangef = Rangef { min: 48.0, max: 2000.0 };
/// The most distance between the edge and the pointer, and the most change of a width
/// that is not the drag. The accessibility tree gives each rect in whole pixels.
const NEAR: f32 = 1.0;
/// The moves of one drag. `mouse.rs` has drags in moves of one point.
const MOVES: u16 = 6;
/// The frames that the window draws after each move with no new input, as a window does
/// for a repaint of some other cause while the user holds the pointer.
const QUIET_FRAMES: usize = 4;

/// The span of each column whose title shows, from the rect of the title.
pub(crate) fn spans(window: &Window) -> Vec<(&'static str, Rangef)> {
    let span = |title: &&'static str| {
        let node = window.query_by_role_and_label(Role::Button, title)?;
        Some((*title, node.rect().x_range()))
    };
    TITLES.iter().filter_map(span).collect()
}

pub(crate) fn span_of(spans: &[(&str, Rangef)], title: &str) -> Option<Rangef> {
    spans.iter().find(|(name, _)| *name == title).map(|(_, span)| *span)
}

/// The width of the column with this title in whole points, as the state file keeps it.
/// Its title must show.
pub(crate) fn width(window: &Window, title: &str) -> f32 {
    window.get_by_role_and_label(Role::Button, title).rect().width().round()
}

/// The right end of the rows: the right edge of the window, less the scroll bar.
pub(crate) fn table_right(window: &Window) -> f32 {
    window.ctx.content_rect().right() - window.ctx.global_style().spacing.scroll.allocated_width()
}

/// A drag of the edge at the right of one title that holds the pointer button.
pub(crate) struct Drag {
    title: &'static str,
    /// Where the pointer took the edge.
    start: egui::Pos2,
    /// The span of the column at the start.
    span: Rangef,
    /// The span of each column whose title showed at the start.
    before: Vec<(&'static str, Rangef)>,
    /// The widths that the column can have.
    widths: Rangef,
}

impl Drag {
    /// Takes the edge at the right of the title of `title` with the pointer.
    pub(crate) fn start(window: &mut Window, title: &'static str) -> Self {
        let before = spans(window);
        let span = span_of(&before, title).unwrap_or_else(|| panic!("the title {title} shows"));
        let y = window.get_by_role_and_label(Role::Button, title).rect().center().y;
        let start = pos2(span.max, y);
        window.hover_at(start);
        window.run();
        window.drag_at(start);
        window.run();
        Self { title, start, span, before, widths: WIDTHS }
    }

    /// Sets the widest width that the column can have in this drag.
    pub(crate) fn with_widest(self, widest: f32) -> Self {
        Self { widths: Rangef::new(WIDTHS.min, widest), ..self }
    }

    /// Moves the pointer to `by` points from the start. Then the edge is at the pointer
    /// inside the limits of a width, the left edge of the column did not move, and each
    /// other column has the width that it had at the start.
    pub(crate) fn move_to(&self, window: &mut Window, by: f32) {
        let (title, start, span) = (self.title, self.start, self.span);
        let pointer = start + egui::vec2(by, 0.0);
        window.hover_at(pointer);
        window.run();
        window.run_steps(QUIET_FRAMES);
        let now = spans(window);
        let shown = span_of(&now, title).unwrap_or_else(|| panic!("the title {title} shows during its drag"));
        let width = (span.span() + by).clamp(self.widths.min, self.widths.max);
        let at = format!("{title} after a move of {by:+} points");
        assert!((shown.min - span.min).abs() <= NEAR, "{at}: its left edge moved from {} to {}", span.min, shown.min);
        assert!(
            (shown.max - (span.min + width)).abs() <= NEAR,
            "{at}: its right edge is at {}, and the pointer at {}",
            shown.max,
            pointer.x
        );
        for (other, was) in self.before.iter().filter(|(other, _)| *other != title) {
            if let Some(is) = span_of(&now, other) {
                let (was, is) = (was.span(), is.span());
                assert!((was - is).abs() <= NEAR, "{at}: {other} changed its width from {was} to {is}");
            }
        }
    }

    /// Lets the edge go at `by` points from the start.
    pub(crate) fn end(self, window: &mut Window, by: f32) {
        window.drop_at(self.start + egui::vec2(by, 0.0));
        window.run();
    }
}

/// Drags the edge at the right of the title of `title` by `by` points, in some moves. After
/// each move, the edge is at the pointer, the left edge of the column did not move, and each
/// other column has the width that it had before the drag.
pub(crate) fn drag(window: &mut Window, title: &'static str, by: f32) {
    let drag = Drag::start(window, title);
    for step in 1..=MOVES {
        drag.move_to(window, by * f32::from(step) / f32::from(MOVES));
    }
    drag.end(window, by);
}

/// Drags the edge of each column whose edge shows, in this sequence of titles, by `by`
/// points and back. An edge that the drag would move out of the table, or under the first
/// column, which does not scroll to the side, stays.
fn drag_each(window: &mut Window, titles: impl Iterator<Item = &'static str>, by: f32, right: f32) {
    for title in titles {
        let now = spans(window);
        let Some(span) = span_of(&now, title) else {
            continue;
        };
        let first = span_of(&now, TITLES[0]).map_or(0.0, |first| first.max);
        if span.max + by.abs() > right || (title != TITLES[0] && span.max - by.abs() < first) {
            continue;
        }
        drag(window, title, by);
        drag(window, title, -by);
    }
}

/// Drags each edge to the right and back from left to right, and then to the left and
/// back from right to left.
fn drag_each_way(window: &mut Window, right: f32) {
    drag_each(window, TITLES.into_iter(), 60.0, right);
    drag_each(window, TITLES.into_iter().rev(), -60.0, right);
}

fn resized_in(size: egui::Vec2) -> Window {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    window.set_size(size);
    window.run();
    window
}

#[test]
fn a_drag_in_the_first_frames_moves_the_edge_with_the_pointer() {
    // The builder of the window draws frames until the first rows show, and no frame more.
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    drag(&mut window, "Namespace", 80.0);
    drag(&mut window, "Status", -50.0);
    drag(&mut window, "Name", 40.0);
}

#[test]
fn each_edge_follows_the_pointer_in_a_window_wider_than_the_columns() {
    let mut window = resized_in(egui::vec2(1800.0, 900.0));
    let right = table_right(&window);
    let age = span_of(&spans(&window), "Age").expect("the last column shows");
    assert!(age.max + 300.0 < right, "the columns end at {}, and the window is wider: {right}", age.max);
    drag_each_way(&mut window, right);
}

#[test]
fn each_edge_follows_the_pointer_in_a_window_narrower_than_the_columns() {
    let mut window = resized_in(egui::vec2(900.0, 900.0));
    let right = table_right(&window);
    let all_show = TITLES.iter().all(|title| span_of(&spans(&window), title).is_some_and(|span| span.max <= right));
    assert!(!all_show, "the columns are wider than the window");
    drag_each_way(&mut window, right);
}

/// At the right end of a narrow table, a narrower column would make the content narrower
/// than the place that the table showed. The scroll offset of the table must then not move
/// the columns under the pointer.
#[test]
fn each_edge_follows_the_pointer_in_a_narrow_window_at_the_right_end_of_the_table() {
    let mut window = resized_in(egui::vec2(900.0, 900.0));
    let name = window.get_by_role_and_label(Role::Button, "Name").rect();
    window.hover_at(pos2(name.right() + 100.0, name.bottom() + 100.0));
    window.run();
    let to_the_right = egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-2000.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    };
    window.event(to_the_right);
    until(&mut window, "the right end of the table", |window| {
        span_of(&spans(window), "Namespace").is_none() && span_of(&spans(window), "Age").is_some()
    });
    let right = table_right(&window);
    drag_each_way(&mut window, right);
}

#[test]
fn the_widths_stay_through_new_data_a_sort_a_filter_and_an_other_page() {
    let mut window = resized_in(egui::vec2(1800.0, 900.0));
    drag(&mut window, "Name", 70.0);
    drag(&mut window, "Node", -40.0);
    drag(&mut window, "Age", 50.0);
    let resized = spans(&window);

    let fewer: Vec<_> = pods().into_iter().filter(|pod| pod.metadata.name.as_deref() != Some("crash")).collect();
    window.state_mut().view = view(vec![live("ams-prod", &fewer)]);
    until(&mut window, "the rows of the new data", |window| shows(window, "24 pods"));
    assert_eq!(spans(&window), resized, "after new data");

    window.get_by_role_and_label(Role::Button, "Age").click();
    window.run();
    window.get_by_role_and_label(Role::Button, "Age").click();
    until(&mut window, "the sort by age", |window| {
        crate::pods::column(window, "Name").last().is_some_and(|name| name == "tick-manual-d6v5w")
    });
    assert_eq!(spans(&window), resized, "after a sort");

    window.get_by_label("Filter pods").click();
    window.run();
    window.get_by_label("Filter pods").type_text("-crash");
    until(&mut window, "the rows of the filter", |window| shows(window, "1 of 24 pods"));
    assert_eq!(spans(&window), resized, "after a filter");
    window.state_mut().view = view(vec![live("ams-prod", &[pod("crash")])]);
    until(&mut window, "no row passes the filter", |window| shows(window, "0 of 1 pod"));
    window.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    window.key_press(egui::Key::Backspace);
    until(&mut window, "the one row", |window| shows(window, "1 pod"));
    assert_eq!(spans(&window), resized, "after a table with no row");

    open_page(&mut window, "Connections");
    open_page(&mut window, "Pods");
    until(&mut window, "the rows again", |window| shows(window, "1 pod"));
    assert_eq!(spans(&window), resized, "after a page that hid the table");
}
