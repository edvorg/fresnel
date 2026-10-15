//! The dialog before a save (plan section 11.2, research 08 section 8.3): the object as
//! cluster, namespace, kind and name, and the diff of each changed key. The values of a
//! Secret stay hidden in the diff until the user shows them; the summary of each key
//! still tells a change of the line feed at the end, which a text editor makes easily.
//!
//! The dialog holds the save that it shows, so the save that goes is the one that the user
//! saw: a new read during the dialog does not change it, and the server refuses it as a
//! conflict when the object changed. The confirm rule `type-name` of the cluster asks for
//! its name, as before each other change.

use egui::{Align, Context, Frame, Id, Key, Layout, Margin, Modal, RichText, ScrollArea, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::keys::{KeyObject, KeyWrite, Place, Save, Value};

use super::diff::{self, Line, Mark};
use super::text;
use crate::format::{self, short};
use crate::theme::{self, Palette};
use crate::views::act::TypedName;
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The width of the dialog.
const WIDTH: f32 = 640.0;
/// The height of the list of the keys before it scrolls.
const LIST_HEIGHT: f32 = 420.0;

/// The diff of one key.
struct KeyDiff {
    key: String,
    what: (Tone, &'static str),
    summary: String,
    lines: Vec<Line>,
}

/// What the user chose in the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    Save,
    Cancel,
}

pub(super) struct Confirm {
    save: Save,
    keys: Vec<KeyDiff>,
    show_values: bool,
    typed: TypedName,
}

impl Confirm {
    /// The dialog of a save of a draft on `base`, the read that the save names.
    pub(super) fn new(save: Save, base: &KeyObject) -> Self {
        let keys = save.writes.iter().map(|write| key_diff(write, base.value(&write.key))).collect();
        Self { save, keys, show_values: false, typed: TypedName::default() }
    }

    /// The save of the dialog.
    pub(super) fn save(&self) -> &Save {
        &self.save
    }

    /// Draws the dialog. `color` is the color of the cluster, `secret` true for a Secret.
    /// `refusal` tells why the save cannot go now: its button is off, and the dialog stays
    /// for a cancel. `view` gives the confirm rule of the cluster.
    pub(super) fn show(
        &mut self,
        ctx: &Context,
        (color, secret): (egui::Color32, bool),
        refusal: Option<&str>,
        view: &CoreView,
    ) -> Option<Answer> {
        let modal = Modal::new(Id::new("save keys")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            self.contents(ui, (color, secret), refusal, view)
        });
        let escape = ctx.input(|input| input.key_pressed(Key::Escape));
        modal.inner.or(escape.then_some(Answer::Cancel))
    }

    fn contents(
        &mut self,
        ui: &mut Ui,
        (color, secret): (egui::Color32, bool),
        refusal: Option<&str>,
        view: &CoreView,
    ) -> Option<Answer> {
        let palette = theme::palette(ui);
        let target = &self.save.target;
        let (kind, key) = (target.kind.kind().info().name, &target.key);
        let count = format::count(self.keys.len() as u64, "key");
        ui.label(RichText::new(format!("Save {count} of this {kind}?")).heading());
        ui.horizontal_wrapped(|ui| {
            dot(ui, color);
            let parts = [
                ("Cluster", target.cluster.as_str()),
                ("Namespace", &*key.namespace),
                ("Kind", kind),
                ("Name", &*key.name),
            ];
            for (label, value) in parts {
                ui.label(RichText::new(label).color(palette.text_muted));
                ui.label(RichText::new(short(value, 120)).strong());
            }
        });
        let note = format!("The save sends only these keys, with version {} of the read.", self.save.version);
        ui.label(RichText::new(note).color(palette.text_muted));
        if secret {
            ui.checkbox(&mut self.show_values, "Show the values");
        }
        ui.add_space(6.0);
        let hidden = secret && !self.show_values;
        ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
            for key in &self.keys {
                show_key(ui, palette, key, hidden);
            }
        });
        ui.add_space(8.0);
        let cluster = self.save.target.cluster.clone();
        let typed = self.typed.show(ui, palette, view, std::iter::once(&cluster));
        let refusal = refusal.map(str::to_owned).or(typed);
        if let Some(reason) = &refusal {
            ui.label(RichText::new(reason).color(palette.warn));
        }
        let mut answer = None;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let save =
                ui.add_enabled(refusal.is_none(), egui::Button::new(RichText::new(format!("Save {count}")).strong()));
            widgets::name(&save, &format!("Save {count}"));
            if save.clicked() {
                answer = Some(Answer::Save);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(Answer::Cancel);
            }
        });
        answer
    }
}

fn show_key(ui: &mut Ui, palette: &Palette, key: &KeyDiff, hidden: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(short(&key.key, 120)).monospace().strong());
        let (tone, word) = key.what;
        ui.label(RichText::new(word).color(tone.color(palette)));
        ui.label(RichText::new(&key.summary).color(palette.text_muted));
    });
    if !hidden && !key.lines.is_empty() {
        Frame::new().fill(palette.bg_sunken).inner_margin(Margin::same(6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for line in &key.lines {
                let (sign, color) = match line.mark {
                    Mark::Same => (" ", palette.text_muted),
                    Mark::Removed => ("-", palette.error),
                    Mark::Added => ("+", palette.ok),
                    Mark::Skipped | Mark::NoLineFeed => ("\\", palette.text_faint),
                };
                let text = short(&line.text, 400);
                ui.label(RichText::new(format!("{sign} {text}")).monospace().color(color));
            }
        });
    }
    ui.add_space(6.0);
}

/// The diff of one write, with the value of the read for the key.
fn key_diff(write: &KeyWrite, old: Option<&Value>) -> KeyDiff {
    let new = write.new.as_ref().map(|(_, value)| value);
    let what = match (old, new) {
        (None, _) => (Tone::Ok, "new"),
        (Some(_), None) => (Tone::Warn, "removed"),
        (Some(_), Some(_)) => (Tone::Info, "changed"),
    };
    let lines = match (text_of(old), text_of(new)) {
        (Some(old), Some(new)) => diff::lines(old, new),
        _ => Vec::new(),
    };
    let mut parts = vec![format!("{}, now {}", describe(old), describe(new))];
    if let (Some(old), Some(new)) = (old.filter(|v| v.text().is_some()), new.filter(|v| v.text().is_some())) {
        let ends = |value: &Value| value.bytes().last() == Some(&b'\n');
        match (ends(old), ends(new)) {
            (true, false) => parts.push("the line feed at the end goes away".to_owned()),
            (false, true) => parts.push("a line feed at the end comes".to_owned()),
            _ => {}
        }
        match (old.has_carriage_return(), new.has_carriage_return()) {
            (false, true) => parts.push("carriage returns come".to_owned()),
            (true, false) => parts.push("the carriage returns go away".to_owned()),
            _ => {}
        }
    }
    match (write.old, &write.new) {
        (Some(Place::Data), Some((Place::BinaryData, _))) => parts.push("it moves to binaryData".to_owned()),
        (Some(Place::BinaryData), Some((Place::Data, _))) => parts.push("it moves to data".to_owned()),
        _ => {}
    }
    KeyDiff { key: write.key.clone(), what, summary: parts.join("; "), lines }
}

/// The text of a value for the diff: no key is an empty text. `None` for a binary value.
fn text_of(value: Option<&Value>) -> Option<&str> {
    value.map_or(Some(""), Value::text)
}

/// "no key", "3 lines, 42 bytes", "binary, 1.2 KiB".
fn describe(value: Option<&Value>) -> String {
    match value {
        None => "no key".to_owned(),
        Some(value) => match value.text() {
            Some(text) => format!("{}, {}", format::count(text::lines(text) as u64, "line"), text::size(value.len())),
            None => format!("binary, {}", text::size(value.len())),
        },
    }
}
