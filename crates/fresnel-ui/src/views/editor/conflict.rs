//! The dialog of a conflict (plan section 12.9): the object changed on the server since the
//! read, and the server and the user changed the same lines. The merge took each other
//! change; for each such place the dialog shows the three versions, the read, the text of
//! the user and the server now, and the user keeps one side. The result goes into the
//! editor on the new read, and the user saves again: nothing goes with no question.

use std::sync::Arc;

use egui::{Align, Context, Frame, Id, Key, Label, Layout, Margin, Modal, RichText, ScrollArea, Ui};
use fresnel_core::app::Remote;
use fresnel_core::manifest::merge::{Conflict, Part, Side};

use crate::format::{count, short};
use crate::theme::{self, Palette};
use crate::widgets;

/// The width of the dialog.
const WIDTH: f32 = 760.0;
/// The height of the list of the places before it scrolls.
const LIST_HEIGHT: f32 = 420.0;
/// The most lines of one version that a place shows.
const MOST_LINES: usize = 40;
/// The lines of the merge before a place, for the key that it is in.
const CONTEXT: usize = 2;

/// What the user chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    /// The text of the merge with the chosen side of each place.
    Use(String),
    Cancel,
}

pub(super) struct ConflictDialog {
    remote: Arc<Remote>,
    /// The version of the read of the save.
    base: String,
    /// The side of each place, in the sequence of the text.
    sides: Vec<Side>,
    /// Why the dialog shows: a conflict, or a save whose result is not known.
    heading: String,
}

impl ConflictDialog {
    pub(super) fn new(remote: Arc<Remote>, base: Option<&str>, heading: String) -> Self {
        let sides = vec![Side::Ours; remote.merge.conflicts().count()];
        Self { remote, base: base.unwrap_or("none").to_owned(), sides, heading }
    }

    /// The object on the server that the dialog merges with.
    pub(super) fn remote(&self) -> &Arc<Remote> {
        &self.remote
    }

    pub(super) fn show(&mut self, ctx: &Context) -> Option<Answer> {
        let modal = Modal::new(Id::new("editor conflict")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            self.contents(ui)
        });
        let escape = ctx.input(|input| input.key_pressed(Key::Escape));
        modal.inner.or(escape.then_some(Answer::Cancel))
    }

    fn contents(&mut self, ui: &mut Ui) -> Option<Answer> {
        let palette = theme::palette(ui);
        ui.label(RichText::new(&self.heading).heading());
        let now = self.remote.read.version.as_deref().unwrap_or("none");
        let places = count(self.sides.len() as u64, "place");
        let note = format!(
            "The server and your text changed the same lines in {places}. The editor took each other change of \
             version {now}. Choose the side of each place; the text then goes into the editor on version {now}, \
             and you save it again."
        );
        ui.add(Label::new(RichText::new(note).color(palette.text_muted)).wrap());
        ui.add_space(6.0);
        let (base, now) = (format!("The read, version {}", self.base), format!("The server now, version {now}"));
        ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
            let mut number = 0;
            let mut before = "";
            for part in &self.remote.merge.parts {
                match part {
                    Part::Merged(text) => before = text,
                    Part::Conflict(conflict) => {
                        let side = self.sides.get_mut(number);
                        place(ui, palette, (number, side), (before, conflict), (&base, &now));
                        number += 1;
                    }
                }
            }
        });
        ui.add_space(8.0);
        let mut answer = None;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let take = ui.add(egui::Button::new(RichText::new("Use in the editor").strong()));
            widgets::name(&take, "Use the merge in the editor");
            if take.clicked() {
                let sides = &self.sides;
                answer = Some(Answer::Use(
                    self.remote.merge.text(|number| sides.get(number).copied().unwrap_or(Side::Ours)),
                ));
            }
            if ui.button("Cancel").clicked() {
                answer = Some(Answer::Cancel);
            }
        });
        answer
    }
}

/// One place: the lines of the merge before it, the three versions, and the choice.
fn place(
    ui: &mut Ui,
    palette: &Palette,
    (number, side): (usize, Option<&mut Side>),
    (before, conflict): (&str, &Conflict),
    (base, now): (&str, &str),
) {
    ui.label(RichText::new(format!("Place {}", number + 1)).strong());
    let context: Vec<&str> = before.lines().rev().take(CONTEXT).collect();
    for line in context.into_iter().rev() {
        ui.label(RichText::new(format!("  {}", short(line, 200))).monospace().color(palette.text_faint));
    }
    version(ui, palette, base, &conflict.base, palette.text_muted);
    version(ui, palette, "Yours", &conflict.ours, palette.info);
    version(ui, palette, now, &conflict.theirs, palette.warn);
    if let Some(side) = side {
        ui.horizontal(|ui| {
            let ours = ui.radio_value(side, Side::Ours, "Keep yours");
            widgets::name(&ours, &format!("Keep yours at place {}", number + 1));
            let theirs = ui.radio_value(side, Side::Theirs, "Take the server's");
            widgets::name(&theirs, &format!("Take the server's at place {}", number + 1));
        });
    }
    ui.add_space(8.0);
}

/// The lines of one version under its name. An empty version has no line.
fn version(ui: &mut Ui, palette: &Palette, name: &str, text: &str, color: egui::Color32) {
    ui.label(RichText::new(name).color(color));
    Frame::new().fill(palette.bg_sunken).inner_margin(Margin::same(4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if text.is_empty() {
            ui.label(RichText::new("no line").italics().color(palette.text_faint));
        }
        let lines: Vec<&str> = text.lines().collect();
        for line in lines.iter().take(MOST_LINES) {
            ui.label(RichText::new(short(line, 200)).monospace().color(palette.text));
        }
        if lines.len() > MOST_LINES {
            let more = count((lines.len() - MOST_LINES) as u64, "more line");
            ui.label(RichText::new(more).color(palette.text_faint));
        }
    });
}
