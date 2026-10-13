//! The table with a slow worker and with a worker that fails (plan section 13): frames
//! go on while a computation waits at a gate, and the test changes the data, the filter
//! or the sort in that time. No test waits a fixed time for rows.

use std::cmp::Ordering;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::cluster::ClusterId;
use fresnel_core::net::task::End;

use super::model::{Cell, Computed};
use super::worker::Cancel;
use super::*;
use crate::testing::{LIMIT, TestTasks};

/// The objects of the test table: a UID and a value.
type Data = Arc<Vec<(u32, i64)>>;

/// What the test does to the computations of the worker.
#[derive(Default)]
struct Control {
    /// With a gate, each computation tells the count of its objects and then waits for
    /// one message of the test.
    gate: Mutex<Option<(Sender<usize>, Receiver<()>)>>,
    /// True for a defect in the rows: each computation panics.
    broken: AtomicBool,
}

struct Numbers(Arc<Control>);

const COLUMNS: [Column; 2] = [Column::new("Object", 200.0), Column::new("Value", 80.0).numeric()];

impl Model for Numbers {
    type Data = Data;
    /// The smallest value of a row.
    type Filter = i64;
    type Row = (ObjectId, u32, i64);
    type Summary = ();
    type Scene = ();

    fn columns(&self) -> &'static [Column] {
        &COLUMNS
    }

    /// This computation does not read its cancel mark, as a slow rule of a kind could do.
    fn rows(&self, data: &Data, filter: &i64, _cancel: &Cancel<'_>) -> Option<Computed<Self::Row, ()>> {
        if let Some((started, gate)) = &*self.0.gate.lock().expect("the gate") {
            started.send(data.len()).expect("the test reads each start");
            gate.recv().expect("the test opens the gate");
        }
        assert!(!self.0.broken.load(Relaxed), "a defect in the rows");
        let id = |uid: &u32| ObjectId { cluster: ClusterId::new("test"), uid: uid.to_string().into() };
        let rows = data.iter().filter(|(_, value)| value >= filter).map(|(uid, value)| (id(uid), *uid, *value));
        Some(Computed { rows: rows.collect(), total: data.len(), summary: () })
    }

    fn compare(&self, a: &Self::Row, b: &Self::Row, column: usize) -> Ordering {
        if column == 1 { a.2.cmp(&b.2) } else { a.1.cmp(&b.1) }
    }

    fn tie(&self, a: &Self::Row, b: &Self::Row) -> Ordering {
        a.1.cmp(&b.1)
    }

    fn id<'a>(&self, row: &'a Self::Row) -> &'a ObjectId {
        &row.0
    }

    fn cell<'a>(&self, row: &'a Self::Row, column: usize, _scene: &'a ()) -> Cell<'a> {
        if column == 1 { Cell::new(row.2.to_string()) } else { Cell::new(format!("object-{}", row.1)) }
    }
}

/// What a frame of the test draws: the table, with the data and the filter of the view.
struct View {
    table: Table<Numbers>,
    data: Data,
    serial: u64,
    filter: i64,
    /// What the user did to the cursor after the test set this to `None`. An input of
    /// the test is some events, and each event has a frame of its own.
    acted: Option<Act>,
}

struct Bed {
    window: Harness<'static, View>,
    control: Arc<Control>,
    /// The count of objects of each computation that started. Only with a gate.
    started: Receiver<usize>,
    release: Sender<()>,
    /// One message for each result of the worker.
    woken: Receiver<()>,
    tasks: TestTasks,
}

impl Bed {
    /// A table with these objects. With `gated`, each computation waits for the test.
    fn start(objects: &[(u32, i64)], gated: bool, broken: bool) -> Self {
        let tasks = TestTasks::new();
        let ((started_at, started), (release, gate), (wake, woken)) = (channel(), channel(), channel());
        let control =
            Arc::new(Control { gate: Mutex::new(gated.then_some((started_at, gate))), broken: broken.into() });
        let wake: Wake = Arc::new(move || {
            // The test can end before the thread of its table.
            if wake.send(()).is_err() {}
        });
        let table = Table::new(&tasks.tasks, wake, "Numbers", Numbers(Arc::clone(&control)));
        let view = View { table, data: Arc::new(objects.to_vec()), serial: 1, filter: i64::MIN, acted: None };
        let frame = |ui: &mut egui::Ui, view: &mut View| {
            view.table.request(&view.filter, view.serial, || Arc::clone(&view.data));
            let acted = view.table.show(ui, &(), || "no row passes the filter".to_owned());
            view.acted = acted.or(view.acted);
        };
        let mut window = Harness::builder().with_size([600.0, 400.0]).build_ui_state(frame, view);
        window.step();
        Self { window, control, started, release, woken, tasks }
    }

    /// New objects, as a new snapshot of a store gives them, and one frame.
    fn set(&mut self, objects: &[(u32, i64)]) {
        let view = self.window.state_mut();
        view.data = Arc::new(objects.to_vec());
        view.serial += 1;
        self.window.step();
    }

    fn started(&self) -> usize {
        self.started.recv_timeout(LIMIT).expect("a computation starts")
    }

    /// Lets the computation at the gate end and waits for its result in the mailbox.
    fn finish(&self) {
        self.release.send(()).expect("the worker waits at the gate");
        self.result();
    }

    fn result(&self) {
        self.woken.recv_timeout(LIMIT).expect("the worker wakes the UI for a result");
    }

    /// The first column of the rows on the screen, from the top.
    fn shown(&self) -> Vec<String> {
        let mut rows: Vec<(f32, String)> = self
            .window
            .query_all_by_label_contains("object-")
            .map(|node| (node.rect().top(), node.accesskit_node().value().unwrap_or_default()))
            .collect();
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        rows.into_iter().map(|(_, label)| label).collect()
    }

    fn click(&mut self, label: &str, modifiers: egui::Modifiers) -> Option<Act> {
        self.window.state_mut().acted = None;
        self.window.get_by_label(label).click_modifiers(modifiers);
        self.window.step();
        self.window.state().acted
    }

    fn key(&mut self, modifiers: egui::Modifiers, key: egui::Key) -> Option<Act> {
        self.window.state_mut().acted = None;
        self.window.key_press_modifiers(modifiers, key);
        self.window.step();
        self.window.state().acted
    }

    fn cursor(&self) -> Option<String> {
        self.window.state().table.selection().cursor().map(|id| id.uid.to_string())
    }

    fn selected(&self) -> usize {
        self.window.state().table.selection().len()
    }

    fn progress(&self) -> String {
        format!("{:?}", self.window.state().table.progress())
    }
}

fn objects(range: std::ops::RangeInclusive<u32>) -> Vec<(u32, i64)> {
    range.map(|uid| (uid, i64::from(uid) * 10)).collect()
}

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

/// Plan section 11.2: each sort has a fixed last key.
#[test]
fn a_click_on_a_title_sorts_and_rows_with_equal_values_keep_one_sequence() {
    let mut bed = Bed::start(&[(1, 5), (2, 5), (3, 1), (4, 9)], false, false);
    bed.result();
    bed.window.step();
    let sort_by_value = |bed: &mut Bed| {
        bed.window.get_by_role_and_label(egui::accesskit::Role::Button, "Value").click();
        bed.window.step();
        bed.window.step();
        bed.result();
        bed.window.step();
        bed.shown()
    };
    assert_eq!(sort_by_value(&mut bed), ["object-3", "object-1", "object-2", "object-4"]);
    assert_eq!(sort_by_value(&mut bed), ["object-4", "object-1", "object-2", "object-3"], "1 stays before 2");

    bed.window.state_mut().filter = 100;
    bed.window.step();
    bed.result();
    bed.window.step();
    assert!(bed.window.query_by_label("no row passes the filter").is_some(), "the view tells why no row shows");
}

#[test]
fn clicks_and_keys_select_objects_and_escape_clears_the_selection() {
    let mut bed = Bed::start(&objects(1..=40), false, false);
    bed.result();
    bed.window.step();
    bed.click("object-2", egui::Modifiers::NONE);
    bed.click("object-4", egui::Modifiers::SHIFT);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (3, Some("4")));
    bed.click("object-3", egui::Modifiers::COMMAND);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (2, Some("3")), "Cmd removes one object");
    bed.click("object-1", egui::Modifiers::NONE);

    assert_eq!(bed.key(egui::Modifiers::NONE, egui::Key::ArrowDown), Some(Act::Cursor));
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (1, Some("2")));
    bed.key(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    bed.key(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (3, Some("4")), "Shift makes a range from the anchor");
    bed.key(egui::Modifiers::SHIFT, egui::Key::ArrowUp);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (2, Some("3")));

    // The last row is far below the window. The table scrolls to the row of the cursor.
    assert!(bed.window.query_by_label("object-40").is_none());
    bed.key(egui::Modifiers::NONE, egui::Key::End);
    bed.window.step();
    assert_eq!(bed.cursor().as_deref(), Some("40"));
    assert!(bed.window.query_by_label("object-40").is_some(), "the row of the cursor shows");
    assert_eq!(bed.key(egui::Modifiers::NONE, egui::Key::Home), Some(Act::Cursor));
    assert_eq!(bed.key(egui::Modifiers::COMMAND, egui::Key::A), None);
    assert_eq!((bed.selected(), bed.cursor().as_deref()), (40, Some("1")), "Cmd+A selects each row");
    assert_eq!(bed.key(egui::Modifiers::NONE, egui::Key::Escape), Some(Act::Cleared));
    assert_eq!((bed.selected(), bed.cursor()), (0, None));
}
