//! The dialog before a create or an apply of a text (plan sections 11.2 and 12.9): each
//! document as cluster, namespace, kind and name, each document that does not go with its
//! reason, and for a force each field that it takes from an other manager.
//!
//! A read-only cluster and the confirm rule `type-name` keep the button off in each frame,
//! as before each other change, and so does a count of documents over the free places of
//! the list of jobs.

use egui::{Align, Color32, Context, Id, Key, Label, Layout, Modal, RichText, ScrollArea, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::cluster::ClusterId;

use super::plan::{Item, Way};
use crate::format::{count, short};
use crate::theme;
use crate::views::act::{TypedName, over_room};
use crate::widgets::{self, dot};

/// The width of the dialog.
const WIDTH: f32 = 640.0;
/// The height of the list of the documents before it scrolls.
const LIST_HEIGHT: f32 = 320.0;

/// What the user chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    Run,
    Cancel,
}

pub(super) struct Confirm {
    pub(super) way: Way,
    cluster: ClusterId,
    namespace: String,
    pub(super) items: Vec<Item>,
    /// The fields of each item of a force, in the sequence of the items.
    fields: Vec<Vec<String>>,
    typed: TypedName,
}

impl Confirm {
    pub(super) fn new(way: Way, (cluster, namespace): (ClusterId, String), items: Vec<Item>) -> Self {
        Self { way, cluster, namespace, items, fields: Vec::new(), typed: TypedName::default() }
    }

    /// The dialog of a force of the items of an apply, with the fields of the conflict of
    /// each one.
    pub(super) fn force(target: (ClusterId, String), items: Vec<(Item, Vec<String>)>) -> Self {
        let (items, fields): (Vec<Item>, _) = items.into_iter().unzip();
        let items = items.into_iter().map(Item::forced).collect();
        Self { fields, ..Self::new(Way::Force, target, items) }
    }

    /// The target cluster and namespace of the dialog.
    pub(super) fn target(&self) -> (ClusterId, String) {
        (self.cluster.clone(), self.namespace.clone())
    }

    pub(super) fn show(&mut self, ctx: &Context, color: Color32, view: &CoreView) -> Option<Answer> {
        let modal = Modal::new(Id::new("create from yaml")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            self.contents(ui, color, view)
        });
        let escape = ctx.input(|input| input.key_pressed(Key::Escape));
        modal.inner.or(escape.then_some(Answer::Cancel))
    }

    fn contents(&mut self, ui: &mut Ui, color: Color32, view: &CoreView) -> Option<Answer> {
        let palette = theme::palette(ui);
        let going = self.items.iter().filter(|item| item.order.is_ok()).count();
        let what = count(going as u64, "object");
        let heading = match self.way {
            Way::Create => format!("Create {what} in {}?", self.cluster),
            Way::Apply => format!("Apply {what} to {}?", self.cluster),
            Way::Force => format!("Apply {what} with force to {}?", self.cluster),
        };
        ui.label(RichText::new(heading).heading());
        ui.horizontal(|ui| {
            dot(ui, color);
            ui.label(RichText::new("Cluster").color(palette.text_muted));
            ui.label(RichText::new(self.cluster.as_str()).strong());
            ui.label(RichText::new("Namespace").color(palette.text_muted));
            ui.label(RichText::new(short(&self.namespace, 80)).strong());
        });
        let note = match self.way {
            Way::Create => "Each object is a job of the Operations tab, and each job waits for the one before it.",
            Way::Apply => {
                "A server-side apply with the field manager fresnel: the server merges the fields of each document \
                 into its object, or makes the object. A field that an other manager set is a conflict."
            }
            Way::Force => "The force takes each of these fields from the manager that set it.",
        };
        ui.add(Label::new(RichText::new(note).color(palette.text_muted)).wrap());
        ui.add_space(6.0);
        ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
            for (index, item) in self.items.iter().enumerate() {
                match &item.order {
                    Ok(_) => ui.label(RichText::new(short(&item.label, 160)).strong()),
                    Err(reason) => {
                        ui.label(RichText::new(format!("{}: {reason}", short(&item.label, 160))).color(palette.warn))
                    }
                };
                for field in self.fields.get(index).into_iter().flatten() {
                    ui.label(RichText::new(format!("   {}", short(field, 200))).monospace().color(palette.text_muted));
                }
            }
        });
        ui.add_space(8.0);
        let typed = self.typed.show(ui, palette, view, std::iter::once(&self.cluster));
        let refusal = view.config.refusal(&self.cluster).or(typed).or_else(|| over_room(going, view.jobs.room()));
        if let Some(reason) = &refusal {
            ui.add(Label::new(RichText::new(reason).color(palette.warn)).wrap());
        }
        let mut answer = None;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let words = format!("{} {}", self.way.verb(), count(going as u64, "object"));
            let run = ui.add_enabled(refusal.is_none() && going > 0, egui::Button::new(RichText::new(&words).strong()));
            widgets::name(&run, &words);
            if run.clicked() {
                answer = Some(Answer::Run);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(Answer::Cancel);
            }
        });
        answer
    }
}
