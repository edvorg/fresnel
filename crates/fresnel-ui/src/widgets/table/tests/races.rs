//! The table with a slow worker and with a worker that fails (plan section 13): what
//! shows while a computation waits at its gate, and after a panic of the thread.

use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::net::task::End;

use super::{Bed, objects};
use crate::testing::LIMIT;
use crate::widgets::table::Act;

/// Defect A1 of the prototype: with snapshots faster than the sort, the table showed
/// the first rows for all time. Here each computation ends, and the newest data is next.
#[test]
fn a_stream_of_snapshots_gives_new_rows_and_the_cursor_stays_on_its_object() {
    let mut bed = Bed::start(&objects(1..=3), true, false);
    assert_eq!((bed.started(), bed.shown()), (3, vec![]));
    assert!(bed.window.query_by_label("The rows are computed…").is_some(), "no empty table before the first rows");
    bed.finish();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-1", "object-2", "object-3"]);
    assert_eq!(bed.click("object-2", egui::Modifiers::NONE), Some(Act::Cursor));
    assert_eq!(bed.cursor().as_deref(), Some("2"));

    // Four snapshots arrive while the worker computes the rows of the first of them.
    bed.set(&[(9, 5), (1, 10), (2, 20), (3, 30)]);
    assert_eq!(bed.started(), 4);
    for last in 10..=12 {
        bed.set(&[vec![(9, 5)], objects(1..=3), objects(10..=last)].concat());
    }
    assert_eq!(bed.shown(), ["object-1", "object-2", "object-3"], "the rows stay while the worker computes");
    bed.finish();
    bed.window.step();
    assert_eq!(bed.shown().len(), 4, "the computation in progress gave its rows: new data did not stop it");
    assert_eq!(bed.started(), 7, "the next computation is of the newest snapshot");
    bed.finish();
    bed.window.step();
    assert_eq!(bed.shown().len(), 7);
    assert_eq!(bed.shown()[..3], ["object-1", "object-2", "object-3"]);
    assert_eq!(bed.cursor().as_deref(), Some("2"), "the cursor is an object, and its row moved");
    assert_eq!(bed.progress(), "Current");
    assert!(bed.tasks.reports.try_recv().is_err(), "no run of the worker ended");
}

/// Defects A2 and A7 of the prototype: the UI put a result into a state that changed
/// after the request. A result of an older filter or sort must never show.
#[test]
fn a_result_of_an_older_filter_or_sort_never_shows() {
    let mut bed = Bed::start(&objects(1..=3), true, false);
    bed.started();
    // The rows of the first request are in the mailbox, and no frame took them yet.
    bed.finish();
    bed.window.state_mut().filter = 25;
    bed.window.step();
    assert_eq!(bed.shown(), Vec::<String>::new(), "the rows of the old filter have two rows too many");
    assert_eq!(bed.started(), 3);
    bed.finish();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-3"]);

    // The same for the sort: a click on a title while the worker computes for the old sort.
    bed.window.state_mut().filter = 0;
    bed.window.step();
    bed.started();
    bed.finish();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-1", "object-2", "object-3"]);
    bed.set(&objects(1..=4));
    bed.started();
    // The last frame of the click turns the sort. Then the rows of the old sort arrive,
    // and the next frame finds them in the mailbox.
    bed.window.get_by_role_and_label(egui::accesskit::Role::Button, "Object").click();
    bed.window.step();
    bed.finish();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-1", "object-2", "object-3"], "object-4 is of a result for the sort before");
    assert_eq!(bed.started(), 4, "the computation for the new sort");
    bed.finish();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-4", "object-3", "object-2", "object-1"]);
}

/// Defect A3 of the prototype, and design rules 1 and 5: a worker that fails keeps the
/// rows on the screen, tells why, and gives new rows after its restart.
#[test]
fn a_worker_panic_keeps_the_rows_and_tells_why_and_new_rows_come_after_the_restart() {
    let mut bed = Bed::start(&objects(1..=3), false, false);
    bed.result();
    bed.window.step();
    assert_eq!(bed.shown().len(), 3);

    bed.control.broken.store(true, Relaxed);
    bed.set(&objects(1..=5));
    let report = bed.tasks.reports.recv_timeout(LIMIT).expect("the report of the run that ended");
    assert_eq!(report.task.to_string(), "table rows [Numbers]");
    assert_eq!(report.end, End::Panicked("a defect in the rows".to_owned()));
    // The worker fails at each start, so some frame finds it in its pause.
    let started = Instant::now();
    while !bed.progress().starts_with("Failed") {
        assert!(started.elapsed() < LIMIT, "the table tells the fault");
        bed.window.step();
    }
    assert_eq!(bed.progress(), r#"Failed("panicked: a defect in the rows; it starts again")"#);
    assert_eq!(bed.shown().len(), 3, "the rows from before the fault stay");

    bed.control.broken.store(false, Relaxed);
    bed.result();
    bed.window.step();
    assert_eq!((bed.shown().len(), bed.progress().as_str()), (5, "Current"), "the request of the panic got its rows");
}

#[test]
fn a_table_with_no_rows_yet_shows_the_fault_of_its_worker_and_not_an_empty_list() {
    let mut bed = Bed::start(&objects(1..=3), false, true);
    let text = "The rows are not computed: the worker thread panicked: a defect in the rows; it starts again";
    let started = Instant::now();
    while bed.window.query_by_label(text).is_none() {
        assert!(started.elapsed() < LIMIT, "the table tells the fault");
        bed.window.step();
    }
    assert!(bed.window.query_by_label("no row passes the filter").is_none());
    bed.control.broken.store(false, Relaxed);
    bed.result();
    bed.window.step();
    assert_eq!(bed.shown().len(), 3);
}

/// The rows on the screen are older than the objects while the worker computes. A click
/// or a key on the row of an object that went away must not select it: the view removed
/// the object from the selection one time, at its change, and nothing would do it again.
#[test]
fn a_click_and_a_key_on_older_rows_select_no_object_that_went_away() {
    let mut bed = Bed::start(&objects(1..=4), true, false);
    bed.started();
    bed.finish();
    bed.window.step();
    bed.click("object-1", egui::Modifiers::NONE);

    // Objects 2 and 3 go away. Their rows stay on the screen while the worker computes.
    bed.set(&[(1, 10), (4, 40)]);
    assert_eq!((bed.started(), bed.shown().len()), (2, 4));
    assert_eq!(bed.click("object-2", egui::Modifiers::NONE), Some(Act::Cursor));
    assert_eq!((bed.selected(), bed.cursor()), (0, None), "a click on a row with no object selects nothing");
    bed.click("object-1", egui::Modifiers::NONE);
    bed.click("object-4", egui::Modifiers::SHIFT);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (2, Some("4")), "the range has the objects 1 and 4");
    bed.key(egui::Modifiers::COMMAND, egui::Key::A);
    assert_eq!(bed.selected(), 2, "Cmd+A selects each object that the rows still have");
    bed.click("object-1", egui::Modifiers::NONE);
    bed.key(egui::Modifiers::NONE, egui::Key::ArrowDown);
    assert_eq!((bed.selected(), bed.cursor()), (0, None), "the key moved the cursor to the row of object 2");

    bed.finish();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-1", "object-4"]);
    // The rows are current again, so each row is the row of an object.
    bed.key(egui::Modifiers::COMMAND, egui::Key::A);
    assert_eq!(bed.selected(), 2);
}

/// Plan section 7.3: a view that is not painted holds no rows. A selection would keep
/// each object that goes away in that time, so it goes too.
#[test]
fn a_table_at_rest_has_no_rows_and_no_selection_and_its_next_frame_asks_for_rows() {
    let mut bed = Bed::start(&objects(1..=3), true, false);
    bed.started();
    bed.finish();
    bed.window.step();
    bed.click("object-2", egui::Modifiers::NONE);
    assert_eq!((bed.shown().len(), bed.selected()), (3, 1));

    let table = &mut bed.window.state_mut().table;
    table.rest();
    // A second call finds nothing to give up.
    table.rest();
    assert!(table.rows().is_none() && bed.selected() == 0 && bed.cursor().is_none());

    bed.window.step();
    assert_eq!(bed.started(), 3, "the frame asks the worker for the rows again");
    assert!(bed.window.query_by_label("The rows are computed…").is_some(), "no empty table before the rows");
    bed.finish();
    bed.window.step();
    assert_eq!((bed.shown().len(), bed.progress().as_str()), (3, "Current"));
}

/// A second fault during the recovery: the user sets a new filter in the pause after a
/// panic, and the thread fails again for the new request. The rows from before the first
/// fault stay with the reason, and the rows of the new filter come after the defect is gone.
#[test]
fn a_second_panic_for_a_new_filter_keeps_the_rows_and_the_rows_of_that_filter_come_then() {
    let mut bed = Bed::start(&objects(1..=3), false, false);
    bed.result();
    bed.window.step();
    bed.control.broken.store(true, Relaxed);
    bed.set(&objects(1..=5));
    let ended = |bed: &Bed| bed.tasks.reports.recv_timeout(LIMIT).expect("the report of a run that ended").end;
    assert_eq!(ended(&bed), End::Panicked("a defect in the rows".to_owned()));

    bed.window.state_mut().filter = 30;
    bed.window.step();
    // A run in progress can be of the first request. The run after it starts after this
    // frame, so it finds the request of the new filter in the mailbox.
    for _ in 0..2 {
        assert_eq!(ended(&bed), End::Panicked("a defect in the rows".to_owned()));
    }
    let started = Instant::now();
    while !bed.progress().starts_with("Failed") {
        assert!(started.elapsed() < LIMIT, "the table tells the fault");
        bed.window.step();
    }
    assert_eq!(bed.shown(), ["object-1", "object-2", "object-3"], "the rows from before the faults stay");

    bed.control.broken.store(false, Relaxed);
    bed.result();
    bed.window.step();
    assert_eq!(bed.shown(), ["object-3", "object-4", "object-5"]);
    assert_eq!(bed.progress(), "Current");
}
