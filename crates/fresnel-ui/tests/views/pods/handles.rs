//! The handle of each edge at its limits: a press beside the line, the last edge past the
//! right side of the table, the narrowest width, a double click on each edge, a window that
//! changes its size during a drag, a click near an edge, columns that the detail panel hides
//! and shows again, the first column of a narrow window and the scroll bar below the rows.

use egui::accesskit::Role;
use egui::{CursorIcon, Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, TouchPhase, pos2};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::Command;

use crate::data::{live, many_pods};
use crate::fake::{Window, until, view, window_with};
use crate::pods::column;
use crate::pods::mouse::{Grip, MouseDrag, on_pixel, wide_window};
use crate::pods::resize::{TITLES, span_of, spans, table_right, width};

/// The grab radius of a handle on each side of its line (`resize_grab_radius_side`).
const GRAB: f32 = 3.0;

fn cursor_at(window: &mut Window, at: Pos2) -> CursorIcon {
    window.hover_at(at);
    window.run();
    window.output().platform_output.cursor_icon
}

/// The middle of the title of a column, at the height of the bar of the titles.
fn title_y(window: &Window) -> f32 {
    window.get_by_role_and_label(Role::Button, "Name").rect().center().y
}

/// The right edge of the column with this title. Its title must show.
fn edge_of(window: &Window, title: &str) -> f32 {
    span_of(&spans(window), title).unwrap_or_else(|| panic!("the title {title} shows")).max
}

/// A press on the line or one point beside it takes the edge, on the titles and on the
/// rows, at a scale of 1 and of 2. The edge then keeps its distance to the pointer.
#[test]
fn a_press_on_the_line_or_one_point_beside_it_takes_the_edge() {
    for scale in [1.0, 2.0] {
        let mut window = wide_window(scale);
        for (title, grip) in [("Namespace", Grip::Rows), ("Status", Grip::Title), ("Name", Grip::Title)] {
            for beside in [0.0, -1.0, 1.0] {
                let mut drag = MouseDrag::press_beside(&mut window, title, grip, beside);
                drag.slide(&mut window, 12.0);
                drag.slide(&mut window, -12.0);
                drag.release(&mut window);
            }
        }
        assert_eq!(window.state().sent, [], "each drag ended at its start");
    }
}

/// The pointer shows the cursor of a resize where a press takes the edge, and only there.
/// A drag that starts beside the handle changes no width.
#[test]
fn the_cursor_of_a_resize_shows_where_a_press_takes_the_edge() {
    let mut window = wide_window(2.0);
    let before = spans(&window);
    let (y, edge) = (title_y(&window), edge_of(&window, "Cluster"));
    for beside in [-GRAB, GRAB] {
        assert_eq!(cursor_at(&mut window, pos2(edge + beside, y)), CursorIcon::ResizeColumn, "at {beside:+}");
    }
    for beside in [-GRAB - 1.0, GRAB + 1.0] {
        let at = pos2(edge + beside, y);
        assert_ne!(cursor_at(&mut window, at), CursorIcon::ResizeColumn, "at {beside:+}");
        window.drag_at(at);
        window.run();
        for step in 1..=20_u8 {
            window.hover_at(at + egui::vec2(f32::from(step), 0.0));
            window.run();
        }
        window.drop_at(at + egui::vec2(20.0, 0.0));
        window.run();
        assert_eq!(spans(&window), before, "a drag from {beside:+} points beside the edge");
    }
}

/// The width of a column in the last layout that the table sent to the core.
fn saved_width(window: &Window, title: &str) -> Option<u32> {
    match window.state().sent.last() {
        Some(fresnel_core::app::Command::SaveTable { layout, .. }) => layout.widths.get(title).copied(),
        _ => None,
    }
}

/// A scroll of the table to the side by `by` points, with the pointer above the rows.
fn scroll_to_the_side(window: &mut Window, by: f32) {
    let name = window.get_by_role_and_label(Role::Button, "Name").rect();
    window.hover_at(pos2(name.right() + 100.0, name.bottom() + 100.0));
    window.run();
    let modifiers = Modifiers::NONE;
    let delta = egui::vec2(-by, 0.0);
    window.event(Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase: TouchPhase::Move, modifiers });
    window.run();
}

/// The edge of the last column follows the pointer past the right side of the table. The
/// table then scrolls to the side, and the handle takes the edge again at its new place.
#[test]
fn the_last_edge_follows_the_pointer_past_the_right_side_of_the_table() {
    let mut window = wide_window(2.0);
    let age = span_of(&spans(&window), "Age").expect("the last column shows");
    let past = table_right(&window) + 60.0 - age.max;
    let mut drag = MouseDrag::press(&mut window, "Age", Grip::Rows);
    for _ in 0..(past / 20.0).ceil() as u16 {
        drag.frame(&mut window, 20.0, 1);
    }
    let pointer = drag.pointer.x;
    drag.release(&mut window);
    assert_eq!(saved_width(&window, "Age"), Some((pointer - age.min).round() as u32), "the width at the release");

    scroll_to_the_side(&mut window, 2000.0);
    until(&mut window, "the right end of the table", |window| {
        span_of(&spans(window), "Age").is_some_and(|age| age.max <= table_right(window))
    });
    let mut drag = MouseDrag::press(&mut window, "Age", Grip::Title);
    drag.slide(&mut window, -30.0);
    drag.release(&mut window);
}

/// A drag to the left past the narrowest width stops the edge there, and the state file
/// gets the narrowest width.
#[test]
fn a_drag_past_the_narrowest_width_stops_there() {
    let mut window = wide_window(2.0);
    let mut drag = MouseDrag::press(&mut window, "Ready", Grip::Rows);
    drag.slide(&mut window, -30.0);
    drag.frame(&mut window, -200.0, 1);
    drag.release(&mut window);
    assert_eq!((width(&window, "Ready"), saved_width(&window, "Ready")), (48.0, Some(48)));
}

/// Two clicks on the edge at the right of a column, `beside` points from its line. They come
/// one second after the clicks before them: egui counts a click within 0.6 s of the second
/// click before it as a third click.
fn double_click(window: &mut Window, title: &str, y: f32, beside: f32) {
    window.run_steps(60);
    let scale = window.ctx.pixels_per_point();
    let at = pos2(on_pixel(edge_of(window, title) + beside, scale), on_pixel(y, scale));
    window.hover_at(at);
    window.run();
    for _ in 0..2 {
        window.drag_at(at);
        window.run();
        let modifiers = Modifiers::NONE;
        window.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers });
        window.run();
    }
}

/// The texts of the rows on the screen below the title of a column that a cell cuts.
fn cut_texts(window: &Window, title: &str) -> Vec<String> {
    crate::pods::cells(window, title).into_iter().filter(|text| text.ends_with('…')).collect()
}

/// Drags the edge of a column to this width in one long move.
fn set_width(window: &mut Window, title: &'static str, to: f32) {
    let mut drag = MouseDrag::press(window, title, Grip::Title);
    drag.frame(window, to - width(window, title), 1);
    drag.release(window);
}

/// A double click on each edge fits its column to its cells, on the titles and on the rows,
/// at a scale of 2. From the narrowest width and from a wider one, the column gets the same
/// width, and no text of the column is cut. No other column changes its width, and the rows
/// keep their sequence.
#[test]
fn a_double_click_on_each_edge_fits_its_column() {
    let mut window = wide_window(2.0);
    let names = column(&window, "Name");
    let header = window.get_by_role_and_label(Role::Button, "Name").rect();
    for (index, title) in TITLES.into_iter().enumerate() {
        let y = if index % 2 == 0 { header.center().y } else { header.bottom() + 200.0 };
        let beside = if index % 3 == 0 { 1.0 } else { 0.0 };
        let before = spans(&window);
        set_width(&mut window, title, 48.0);
        double_click(&mut window, title, y, beside);
        let fitted = width(&window, title);
        assert!(fitted > 48.0, "{title} after a double click: {fitted}");
        assert_eq!(cut_texts(&window, title), Vec::<String>::new(), "{title} after a double click");
        set_width(&mut window, title, fitted + 100.0);
        double_click(&mut window, title, y, beside);
        assert_eq!(width(&window, title), fitted, "{title} after a double click at a wider width");
        for (other, was) in before.iter().filter(|(other, _)| *other != title) {
            assert_eq!(width(&window, other), was.span().round(), "{other} after a double click on {title}");
        }
    }
    assert_eq!(column(&window, "Name"), names, "the sequence of the rows");
}

/// A window that changes its size during a drag does not move the edge from the pointer.
#[test]
fn a_window_that_changes_its_size_during_a_drag() {
    let mut window = wide_window(2.0);
    let wide = window.ctx.content_rect().size();
    let mut drag = MouseDrag::press(&mut window, "Namespace", Grip::Rows);
    drag.slide(&mut window, 20.0);
    for size in [egui::vec2(1148.0, 900.0), egui::vec2(748.0, 900.0), wide] {
        window.set_size(size);
        window.run();
        drag.check(&window, &format!("in a window of {} points", size.x));
        drag.slide(&mut window, 10.0);
    }
    drag.release(&mut window);
    assert_eq!(width(&window, "Namespace"), 120.0 + 50.0);
}

/// The pointer comes to this place, and the button goes down and up there. Then the
/// window draws frames for half a second, so the next click is no double click.
fn click(window: &mut Window, at: Pos2) {
    window.hover_at(at);
    window.run();
    window.drag_at(at);
    window.run();
    window.drop_at(at);
    window.run();
    window.run_steps(30);
}

/// A click on a title beside the handle of an edge sorts the rows, and a click on the
/// handle does not.
#[test]
fn a_click_beside_the_handle_sorts_and_a_click_on_it_does_not() {
    let mut window = wide_window(2.0);
    let (y, edge) = (title_y(&window), edge_of(&window, "Namespace"));
    let (namespaces, before) = (column(&window, "Namespace"), spans(&window));
    for beside in [0.0, -1.0, 1.0, -GRAB, GRAB] {
        click(&mut window, pos2(edge + beside, y));
        assert_eq!(column(&window, "Namespace"), namespaces, "after a click at {beside:+} points");
        assert_eq!(spans(&window), before, "after a click at {beside:+} points");
    }
    click(&mut window, pos2(edge - GRAB - 2.0, y));
    until(&mut window, "the sort by namespace", |window| {
        let now = column(window, "Namespace");
        now != namespaces && now.is_sorted()
    });
}

/// The detail panel of a pod takes a part of the window, and the columns at the right of the
/// table go out of the table. After the panel closes, they show again with their widths,
/// and their handles take the edges again.
#[test]
fn columns_that_the_detail_panel_hides_show_again_with_their_widths_and_handles() {
    let mut window = window_with(view(vec![live("ams-prod", &many_pods(35))]), egui::Theme::Dark, 2.0);
    window.set_size(egui::vec2(1348.0, 900.0));
    window.run();
    let age = width(&window, "Age");
    let mut drag = MouseDrag::press(&mut window, "Age", Grip::Rows);
    drag.slide(&mut window, 40.0);
    drag.release(&mut window);
    let tables = |window: &Window| -> Vec<Command> {
        let sent = window.state().sent.iter();
        sent.filter(|command| matches!(command, Command::SaveTable { .. })).cloned().collect()
    };
    let (before, sent) = (spans(&window), tables(&window));

    let name = window.get_by_role_and_label(Role::Button, "Name").rect();
    click(&mut window, pos2(name.center().x, name.bottom() + 40.0));
    until(&mut window, "the detail panel", |window| window.query_by_label("Close the details").is_some());
    let narrow = spans(&window);
    assert_eq!(span_of(&narrow, "Age"), None, "the panel hides Age");
    for (title, span) in &narrow {
        assert_eq!(Some(*span), span_of(&before, title), "{title} with the panel");
    }
    window.get_by_label("Close the details").click();
    until(&mut window, "no detail panel", |window| window.query_by_label("Close the details").is_none());
    assert_eq!(spans(&window), before, "after the panel");
    assert_eq!(tables(&window), sent, "the panel changes no width");
    let mut drag = MouseDrag::press(&mut window, "Age", Grip::Title);
    drag.slide(&mut window, -40.0);
    drag.release(&mut window);
    assert_eq!(width(&window, "Age"), age);
}

/// A window narrower than the width of the first column shows that column inside the
/// table. A click on its edge, a move to the right and a drag that Escape ends keep the
/// width of the user: a wider window shows it again, and the state file keeps it.
#[test]
fn a_click_a_move_to_the_right_and_escape_at_a_first_column_that_the_window_cuts_keep_its_width() {
    let mut window = window_with(view(vec![live("ams-prod", &many_pods(35))]), egui::Theme::Dark, 2.0);
    window.set_size(egui::vec2(2600.0, 900.0));
    window.run();
    let mut drag = MouseDrag::press(&mut window, "Name", Grip::Title);
    drag.frame(&mut window, 1780.0, 1);
    drag.release(&mut window);
    assert_eq!(width(&window, "Name"), 2000.0);
    let sent = window.state().sent.clone();

    window.set_size(egui::vec2(900.0, 900.0));
    window.run();
    let cut = width(&window, "Name");
    assert!(cut < 900.0, "the window shows {cut} points of Name");
    let edge = pos2(edge_of(&window, "Name"), title_y(&window));
    click(&mut window, edge);
    let mut drag = MouseDrag::press(&mut window, "Name", Grip::Title);
    drag.frame_with_no_check(&mut window, 1.0);
    drag.frame_with_no_check(&mut window, 30.0);
    assert_eq!(width(&window, "Name"), cut, "a move to the right");
    window.drop_at(drag.pointer);
    window.run();
    let mut drag = MouseDrag::press(&mut window, "Name", Grip::Rows);
    drag.slide(&mut window, -20.0);
    window.key_press(egui::Key::Escape);
    window.run();
    window.drop_at(drag.pointer);
    window.run();
    assert_eq!(width(&window, "Name"), cut, "the narrow window");
    assert_eq!(window.state().sent, sent, "no width changed");
    window.set_size(egui::vec2(2600.0, 900.0));
    window.run();
    assert_eq!(width(&window, "Name"), 2000.0, "the width of the user");
}

/// The place of the scroll bar below the rows: the lowest place of the table, above the
/// handle of the dock.
fn scroll_bar_y(window: &mut Window, x: f32) -> f32 {
    let dock = window.get_by_role_and_label(Role::Button, "Journal").rect().top();
    let mut y = dock;
    while cursor_at(window, pos2(x, y)) == CursorIcon::ResizeVertical {
        y -= 1.0;
    }
    let bar = window.ctx.global_style().spacing.scroll.bar_width;
    y - bar / 2.0
}

/// In a window narrower than the columns, a drag on the scroll bar below the rows scrolls
/// the table, also at the place of an edge. No handle covers the scroll bar.
#[test]
fn a_drag_on_the_scroll_bar_below_the_rows_scrolls_also_at_an_edge() {
    let mut window = window_with(view(vec![live("ams-prod", &many_pods(35))]), egui::Theme::Dark, 2.0);
    window.set_size(egui::vec2(900.0, 900.0));
    window.run();
    let before = spans(&window);
    let edge = edge_of(&window, "Namespace");
    let y = scroll_bar_y(&mut window, edge + 40.0);
    for x in [edge + 40.0, edge] {
        let at = pos2(x, y);
        assert_ne!(cursor_at(&mut window, at), CursorIcon::ResizeColumn, "on the scroll bar at {x}");
        window.drag_at(at);
        window.run();
        for step in 1..=10_u8 {
            window.hover_at(at + egui::vec2(6.0 * f32::from(step), 0.0));
            window.run();
        }
        window.drop_at(at + egui::vec2(60.0, 0.0));
        window.run();
        assert_eq!(span_of(&spans(&window), "Namespace"), None, "a drag at {x} scrolls the table");
        assert_eq!(window.state().sent, [], "a drag at {x} changes no width");
        scroll_to_the_side(&mut window, -2000.0);
        until(&mut window, "the left end of the table", |window| spans(window) == before);
    }
}
