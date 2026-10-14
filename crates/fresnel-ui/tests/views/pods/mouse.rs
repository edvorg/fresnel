//! Drags of an edge as a mouse sends them, in a window as wide as the one of the user: a
//! table of about 1,750 points with 35 rows, at a scale of 1 and of 2 (a Retina display).
//!
//! The pointer comes to the edge, the button goes down on the edge, the pointer moves in
//! many small steps on the pixel grid of the display, and the button goes up. The window
//! draws the frames that egui asks for after each input, as a real window does. In the
//! frame of each move and after the frames that follow it, the edge is within 2 points of
//! the pointer, the left edge of the column did not move, and no other column changed its
//! width.

use egui::accesskit::Role;
use egui::{CursorIcon, Event, Key, Pos2, Rangef, pos2};
use egui_kittest::kittest::Queryable as _;

use crate::data::{live, many_pods};
use crate::fake::{Window, shows, until, view, window_with};
use crate::pods::column;
use crate::pods::resize::{span_of, spans, table_right};
use crate::pods::scenes::pods_of;

/// The most distance between the edge and the pointer.
const NEAR: f32 = 2.0;
/// The most change of a place that does not follow the pointer: the accessibility tree
/// gives each rect in whole pixels.
const STILL: f32 = 0.5;
/// The widths that a column can have.
const WIDTHS: Rangef = Rangef { min: 48.0, max: 2000.0 };
/// The width of the window of the user, with the sidebar at the left of the table.
const WIDE: egui::Vec2 = egui::vec2(1910.0, 1000.0);
const ROWS: usize = 35;

/// The window of the user at this scale, with its 35 rows.
pub(crate) fn wide_window(scale: f32) -> Window {
    let mut window = window_with(view(vec![live("ams-prod", &many_pods(ROWS))]), egui::Theme::Dark, scale);
    window.set_size(WIDE);
    until(&mut window, "the 35 rows", |window| shows(window, "35 pods"));
    let left = span_of(&spans(&window), "Name").expect("the first column").min;
    let width = table_right(&window) - left;
    assert!((1650.0..=1750.0).contains(&width), "the table is {width} points wide");
    window
}

/// Where the button goes down on an edge.
#[derive(Clone, Copy)]
pub(crate) enum Grip {
    /// On the bar of the titles.
    Title,
    /// On the line between two cells, in the middle of the rows.
    Rows,
}

/// One drag of the edge at the right of a column, from the press to the release.
pub(crate) struct MouseDrag {
    title: &'static str,
    scale: f32,
    /// The pointer now.
    pub(crate) pointer: Pos2,
    /// The edge at the press, and the pointer there.
    edge: f32,
    press: f32,
    /// The left edge of the column at the press.
    left: f32,
    /// The width of each other column whose title showed at the press.
    others: Vec<(&'static str, f32)>,
    /// The most distance between the edge and the pointer.
    near: f32,
}

impl MouseDrag {
    /// The pointer comes to the edge at the right of `title`, and the button goes down.
    pub(crate) fn press(window: &mut Window, title: &'static str, grip: Grip) -> Self {
        Self::press_beside(window, title, grip, 0.0)
    }

    /// The button goes down `beside` points to the right of the edge at the right of `title`.
    pub(crate) fn press_beside(window: &mut Window, title: &'static str, grip: Grip, beside: f32) -> Self {
        let scale = window.ctx.pixels_per_point();
        let before = spans(window);
        let span = span_of(&before, title).unwrap_or_else(|| panic!("the title {title} shows"));
        let header = window.get_by_role_and_label(Role::Button, title).rect();
        let y = match grip {
            Grip::Title => header.center().y,
            Grip::Rows => header.bottom() + 300.0,
        };
        let pointer = pos2(on_pixel(span.max + beside, scale), on_pixel(y, scale));
        window.hover_at(pointer);
        window.run();
        window.drag_at(pointer);
        window.run();
        let others = before.iter().filter(|(other, _)| *other != title).map(|(other, span)| (*other, span.span()));
        let (edge, press, left, others) = (span.max, pointer.x, span.min, others.collect());
        Self { title, scale, pointer, edge, press, left, others, near: NEAR }
    }

    /// Moves the pointer by `by` points in `moves` moves of one frame, and draws the frames
    /// that the window draws after that input.
    pub(crate) fn frame(&mut self, window: &mut Window, by: f32, moves: u16) {
        let start = self.pointer.x;
        for step in 1..=moves {
            let x = on_pixel(start + by * f32::from(step) / f32::from(moves), self.scale);
            self.pointer.x = x;
            window.input_mut().events.push(Event::PointerMoved(self.pointer));
        }
        let at = format!("a move to {:+} points", self.pointer.x - self.press);
        window.step();
        self.check(window, &format!("in the frame of {at}"));
        window.run();
        self.check(window, &format!("after {at}"));
    }

    /// Moves the pointer by `by` points in one frame, and checks nothing.
    pub(crate) fn frame_with_no_check(&mut self, window: &mut Window, by: f32) {
        self.pointer.x = on_pixel(self.pointer.x + by, self.scale);
        window.hover_at(self.pointer);
        window.run();
    }

    /// Moves the pointer by `by` points, one point or less in each frame.
    pub(crate) fn slide(&mut self, window: &mut Window, by: f32) {
        let frames = by.abs().ceil() as u16;
        for _ in 0..frames {
            self.frame(window, by / f32::from(frames), 1);
        }
    }

    /// The button goes up where the pointer is.
    pub(crate) fn release(self, window: &mut Window) {
        window.drop_at(self.pointer);
        window.run();
        self.check(window, "after the release");
    }

    pub(crate) fn check(&self, window: &Window, at: &str) {
        let (title, scale) = (self.title, self.scale);
        let at = format!("{title} at scale {scale}, {at}");
        let now = spans(window);
        let shown = span_of(&now, title).unwrap_or_else(|| panic!("{at}: the title shows"));
        assert!(
            (shown.min - self.left).abs() <= STILL,
            "{at}: the column moved {} by {} points, from {} to {}",
            if shown.min < self.left { "left" } else { "right" },
            (shown.min - self.left).abs(),
            self.left,
            shown.min
        );
        let wanted = self.left + (self.edge - self.left + self.pointer.x - self.press).clamp(WIDTHS.min, WIDTHS.max);
        assert!(
            (shown.max - wanted).abs() <= self.near,
            "{at}: the edge is at {}, and the pointer at {} wants it at {wanted}",
            shown.max,
            self.pointer.x
        );
        for (other, was) in &self.others {
            if let Some(is) = span_of(&now, other) {
                assert!(
                    (is.span() - was).abs() <= STILL,
                    "{at}: {other} changed its width from {was} to {}",
                    is.span()
                );
            }
        }
    }
}

/// The place of a pixel edge of a display with this scale: a mouse gives no other place.
pub(crate) fn on_pixel(x: f32, scale: f32) -> f32 {
    (x * scale).round() / scale
}

/// The drag of the report of the user, on a Retina display: the button goes down on the
/// edge right of Namespace, and the pointer moves to the right. The column did not stay:
/// it moved to the left. This test checks only that, and the other tests also the edge.
#[test]
fn the_column_of_a_drag_to_the_right_stays_where_it_is() {
    let mut window = wide_window(2.0);
    for title in ["Namespace", "Cluster", "Node"] {
        let mut drag = MouseDrag::press(&mut window, title, Grip::Rows);
        drag.near = f32::INFINITY;
        drag.slide(&mut window, 60.0);
        drag.release(&mut window);
    }
}

/// Drags the edge of each of these columns to the right and back to the left, in moves of
/// one point or less, one move in each frame.
fn slow_drags(scale: f32) {
    let mut window = wide_window(scale);
    for (title, grip) in [("Namespace", Grip::Rows), ("Cluster", Grip::Title), ("Node", Grip::Rows)] {
        let mut drag = MouseDrag::press(&mut window, title, grip);
        drag.slide(&mut window, 120.0);
        drag.release(&mut window);
        let mut drag = MouseDrag::press(&mut window, title, grip);
        drag.slide(&mut window, -80.0);
        drag.slide(&mut window, 20.5);
        drag.release(&mut window);
    }
}

#[test]
fn slow_mouse_drags_at_scale_2() {
    slow_drags(2.0);
}

#[test]
fn slow_mouse_drags_at_scale_1() {
    slow_drags(1.0);
}

/// A fast mouse moves far between two frames, and a window that is busy gets some moves
/// in one frame.
#[test]
fn fast_mouse_drags_with_long_moves_and_some_moves_in_one_frame() {
    for scale in [1.0, 2.0] {
        let mut window = wide_window(scale);
        let mut drag = MouseDrag::press(&mut window, "Namespace", Grip::Rows);
        drag.frame(&mut window, 300.0, 1);
        drag.frame(&mut window, -420.0, 1);
        drag.frame(&mut window, 37.5, 5);
        drag.frame(&mut window, 61.0, 3);
        drag.release(&mut window);
        let mut drag = MouseDrag::press(&mut window, "Status", Grip::Title);
        drag.frame(&mut window, -90.0, 1);
        drag.frame(&mut window, 400.0, 8);
        drag.release(&mut window);
    }
}

/// New rows arrive during a drag: a second cluster, and then fewer rows.
#[test]
fn new_rows_during_a_mouse_drag() {
    for scale in [1.0, 2.0] {
        let mut window = wide_window(scale);
        let mut drag = MouseDrag::press(&mut window, "Cluster", Grip::Rows);
        drag.slide(&mut window, 30.0);
        let two = vec![live("ams-prod", &many_pods(ROWS)), live("fra-stage", &pods_of("fra"))];
        window.state_mut().view = view(two);
        until(&mut window, "the rows of two clusters", |window| shows(window, "41 pods"));
        drag.check(&window, "after new rows");
        drag.slide(&mut window, 30.0);
        window.state_mut().view = view(vec![live("ams-prod", &many_pods(12))]);
        until(&mut window, "fewer rows", |window| shows(window, "12 pods"));
        drag.check(&window, "after fewer rows");
        drag.slide(&mut window, -45.0);
        drag.release(&mut window);
    }
}

/// Each edge has a handle, also the edge between Name and Namespace: the pointer on the
/// line shows the cursor of a resize, on the titles and on the rows. Away from the line,
/// it does not.
#[test]
fn each_edge_has_a_handle() {
    for scale in [1.0, 2.0] {
        let mut window = wide_window(scale);
        let header = window.get_by_role_and_label(Role::Button, "Name").rect();
        for (title, span) in spans(&window) {
            for (y, place) in [(header.center().y, "title"), (header.bottom() + 200.0, "rows")] {
                let cursor = |window: &mut Window, x: f32| {
                    window.hover_at(pos2(on_pixel(x, scale), y));
                    window.run();
                    window.output().platform_output.cursor_icon
                };
                let at = format!("the edge of {title} at scale {scale}, on the {place}");
                assert_eq!(cursor(&mut window, span.max), CursorIcon::ResizeColumn, "{at}");
                assert_ne!(cursor(&mut window, span.max - 20.0), CursorIcon::ResizeColumn, "left of {at}");
            }
        }
    }
}

/// A click on an edge, a little beside its line, changes no width and does not sort. Escape
/// ends a drag with the width of its start, and no layout goes to the core.
#[test]
fn a_click_on_an_edge_and_escape_during_a_drag_change_no_width() {
    let mut window = wide_window(2.0);
    let (before, names) = (spans(&window), column(&window, "Name"));
    let namespace = span_of(&before, "Namespace").expect("the title shows");
    let header = window.get_by_role_and_label(Role::Button, "Namespace").rect();
    let at = pos2(namespace.max + 3.0, header.center().y);
    window.hover_at(at);
    window.run();
    window.drag_at(at);
    window.run();
    window.drop_at(at);
    window.run();
    assert_eq!(spans(&window), before, "after a click");
    assert_eq!(column(&window, "Name"), names, "the sequence of the rows");

    let mut drag = MouseDrag::press(&mut window, "Cluster", Grip::Rows);
    drag.slide(&mut window, 40.0);
    window.key_press(Key::Escape);
    window.run();
    assert_eq!(spans(&window), before, "after Escape");
    drag.frame_with_no_check(&mut window, 30.0);
    window.drop_at(drag.pointer);
    window.run();
    assert_eq!(spans(&window), before, "a drag after Escape has no effect");
    assert_eq!(window.state().sent, [], "no width changed");
}
