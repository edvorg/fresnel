//! The table on a model with numbers. With a gate, a computation of the worker waits for
//! the test, so frames go on while the test changes the data, the filter or the sort. No
//! test waits a fixed time for rows.
//!
//! - `races`: a slow worker and a worker that fails (plan section 13).
//! - `input`: the sort, the clicks and the keys.

mod input;
mod races;

use std::cmp::Ordering;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::cluster::ClusterId;
use fresnel_core::store::ObjectKey;

use super::model::{Cell, Column, Computed};
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
        let id = |uid: &u32| {
            let key = ObjectKey::new("test", &format!("object-{uid}"));
            ObjectId { cluster: ClusterId::new("test"), key, uid: uid.to_string().into() }
        };
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

    fn has(&self, data: &Data, row: &Self::Row) -> bool {
        data.iter().any(|(uid, _)| *uid == row.1)
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
