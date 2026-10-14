//! The quick search (plan section 12.1): a dialog over the window that a key opens from
//! each page. It searches each kind of each selected cluster in the stores of the app, on a
//! worker thread, and lists each hit with its cluster, kind, namespace and name, and the
//! mark of a store that is not live. The notes below the list name each store that the
//! search did not read, with the reason (weaknesses 5 and 6 of plan section 6).
//!
//! The list starts with the commands of the command table whose label has each word of
//! the text. A hit opens its object, and a command runs.
//!
//! - `request`: what the worker reads, and when the stores changed. `filters`: the menus
//!   of the clusters and the kinds. `list`: the commands and the hits. `notes`: the stores
//!   that the search did not read.

mod filters;
mod list;
mod notes;
mod request;

use egui::{Context, Id, Key, Modal, Modifiers, RichText, TextEdit, Ui};
use fresnel_core::commands::{CommandId, Keys};
use fresnel_core::net::task::{TaskName, Tasks};
use fresnel_core::search::{self, Found};

use self::list::Row;
use self::request::{Request, Stores};
use crate::format;
use crate::shell::{Scene, keys as shell_keys};
use crate::theme::{self, PAD};
use crate::views::objects::Link;
use crate::widgets;
use crate::widgets::worker::{Cancel, Done, RequestId, Wake, Worker};

/// The width of the dialog.
const WIDTH: f32 = 760.0;

/// What the user chose in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Opens the object of a hit.
    Open(Link),
    /// Runs a command.
    Run(CommandId),
}

pub(crate) struct QuickSearch {
    open: bool,
    /// True from the open to the frame that gives the focus to the text field.
    focus: bool,
    query: search::Query,
    /// A number that grows with each change of the query.
    query_serial: u64,
    stores: Stores,
    worker: Worker<Request, Found>,
    asked: Option<RequestId>,
    /// The result on the screen with the identity of its request.
    found: Option<Done<Found>>,
    /// The place of the cursor in the list: the commands, then the hits.
    cursor: usize,
}

impl QuickSearch {
    /// Starts the worker thread. `wake` asks for a repaint for a result.
    pub(crate) fn new(tasks: &Tasks, wake: Wake) -> Self {
        let compute = |request: &Request, cancel: &Cancel<'_>| {
            search::run(&request.query, &request.clusters, &|| cancel.is_set())
        };
        Self {
            open: false,
            focus: false,
            query: search::Query::default(),
            query_serial: 0,
            stores: Stores::default(),
            worker: Worker::start(tasks, TaskName::new("quick search"), wake, compute),
            asked: None,
            found: None,
            cursor: 0,
        }
    }

    /// Opens the dialog with the text and the filters of the last search.
    pub(crate) fn open(&mut self) {
        self.open = true;
        self.focus = true;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Closes the dialog. The worker drops its request: a request holds a snapshot of each
    /// store (plan section 7.3).
    fn close(&mut self) {
        self.open = false;
        self.worker.clear();
        (self.asked, self.found, self.cursor) = (None, None, 0);
        self.stores = Stores::default();
    }

    /// Draws the dialog while it is open, and gives what the user chose.
    pub(crate) fn show(&mut self, ctx: &Context, scene: &Scene<'_>, keys: &Keys) -> Option<Choice> {
        if !self.open {
            return None;
        }
        self.ask(scene);
        let modal = Modal::new(Id::new("quick search")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            self.contents(ui, scene, keys)
        });
        let close = modal.should_close();
        let choice = modal.inner;
        if choice.is_some() || close {
            self.close();
        }
        choice
    }

    /// Sends a request for the query and the stores of this frame, if they are new, and
    /// takes a result of the worker.
    fn ask(&mut self, scene: &Scene<'_>) {
        let id = RequestId { query: self.query_serial, data: self.stores.serial(scene.view) };
        if self.asked != Some(id) {
            self.worker.submit(id, Request::new(&self.query, scene.view));
            self.asked = Some(id);
        }
        if let Some(done) = self.worker.take() {
            // A result of an older query does not fit the text on the screen.
            if done.id.query == self.query_serial {
                self.found = Some(done);
            }
        }
    }

    fn contents(&mut self, ui: &mut Ui, scene: &Scene<'_>, keys: &Keys) -> Option<Choice> {
        let palette = theme::palette(ui);
        let rows = list::rows(self.found(), &self.query.text, |id| shell_keys::text(ui.ctx(), keys, id));
        let (moved, enter) = self.keys(ui, rows.len());
        let found = self.found.as_ref().map(|done| &done.output);
        let field = TextEdit::singleline(&mut self.query.text)
            .hint_text("A name, a label, an image, a node or an IP address")
            .desired_width(f32::INFINITY);
        let field = ui.add(field);
        widgets::name(&field, "Search text");
        if std::mem::take(&mut self.focus) {
            field.request_focus();
        }
        ui.add_space(PAD);
        // A worker that ended has a state of its own, so old hits never look current.
        let failure = (self.asked != self.found.as_ref().map(|done| done.id)).then(|| self.worker.failure()).flatten();
        let changed = ui.horizontal(|ui| {
            let changed = filters::show(ui, &mut self.query, scene.view);
            match &failure {
                Some(failure) => {
                    let text = format!("The hits are not current: the worker thread {failure}");
                    ui.label(RichText::new(text).color(palette.error))
                }
                None => ui.label(RichText::new(count_text(found, &self.query.text)).color(palette.text_muted)),
            };
            changed
        });
        if field.changed() || changed.inner {
            self.query_serial += 1;
            self.cursor = 0;
        }
        ui.add_space(PAD);
        let clicked = list::show(ui, scene, &rows, self.cursor, moved);
        if let Some(found) = found {
            notes::show(ui, palette, &found.unread);
        }
        let row = clicked.or_else(|| enter.then_some(self.cursor)).and_then(|index| rows.get(index));
        row.map(Row::choice)
    }

    fn found(&self) -> Option<&Found> {
        self.found.as_ref().map(|done| &done.output)
    }

    /// The keys of the list: the arrows move the cursor, and Enter chooses its row. They
    /// come before the text field, which would take them. Gives true for a move of the
    /// cursor and for Enter.
    fn keys(&mut self, ui: &Ui, rows: usize) -> (bool, bool) {
        let [down, up, enter] = [Key::ArrowDown, Key::ArrowUp, Key::Enter]
            .map(|key| ui.input_mut(|input| input.consume_key(Modifiers::NONE, key)));
        let last = rows.saturating_sub(1);
        if down {
            self.cursor += 1;
        }
        if up {
            self.cursor = self.cursor.saturating_sub(1);
        }
        self.cursor = self.cursor.min(last);
        (down || up, enter)
    }
}

/// "12 hits", "200 of 1,532 hits": the hits on the screen and all of them.
fn count_text(found: Option<&Found>, text: &str) -> String {
    let Some(found) = found else {
        return "The search reads the stores…".to_owned();
    };
    if text.trim().is_empty() {
        return String::new();
    }
    let total = found.total as u64;
    if found.total > found.hits.len() {
        format!("{} of {}", format::thousands(found.hits.len() as u64), format::count(total, "hit"))
    } else {
        format::count(total, "hit")
    }
}
