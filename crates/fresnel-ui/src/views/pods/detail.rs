//! The detail panel of one pod: its metadata and its Events (plan section 11.2).
//!
//! The panel reads the pod from the snapshot of each frame, so it shows each change. In
//! the UI prototype, the panel built its data one time and showed a deleted object with
//! each button. Here a pod that left its store shows as deleted, with its last state.
//!
//! - `events`: the Events of the pod.

mod events;

use std::sync::Arc;
use std::time::SystemTime;

use egui::{Align, Button, Frame, Label, Layout, Margin, RichText, ScrollArea, Sense, Ui, WidgetInfo, WidgetType};
use fresnel_core::app::ClusterRow;
use fresnel_core::kinds::PodStatus;
use fresnel_core::store::Snapshot;
use k8s_openapi::api::core::v1::Pod;

use super::notes::old_since;
use super::rows::{PodRow, age, uid};
use crate::shell::Scene;
use crate::theme::{self, Palette, SMALL};
use crate::widgets::table::cells::ring;
use crate::widgets::table::model::ObjectId;
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The width of the labels of the metadata.
const LABEL_WIDTH: f32 = 104.0;
/// The most characters of the value of an annotation. A value can be a full manifest.
const VALUE_LIMIT: usize = 240;

/// What the store of the cluster knows of the pod of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Presence {
    Present,
    /// The store had a complete list, and the pod is not in it.
    Deleted,
    /// The store has no complete list now, so it does not know the pod. That is not a
    /// deleted pod (design rule 5).
    NotLoaded,
}

/// The pod of the panel, with its last state.
pub(super) struct Subject {
    pub(super) id: ObjectId,
    pod: Arc<Pod>,
    presence: Presence,
}

impl Subject {
    pub(super) fn of(row: &PodRow) -> Self {
        Self { id: row.id.clone(), pod: Arc::clone(&row.pod), presence: Presence::Present }
    }

    /// Reads the pod from the snapshot of this frame. A pod with the same name and an
    /// other UID is an other object.
    pub(super) fn refresh(&mut self, pods: &Snapshot<Pod>) {
        let found = pods.items.get(&self.id.key).filter(|pod| uid(pod) == &*self.id.uid);
        self.presence = match found {
            Some(pod) => {
                self.pod = Arc::clone(pod);
                Presence::Present
            }
            None if pods.listed => Presence::Deleted,
            None => Presence::NotLoaded,
        };
    }
}

/// Draws the panel. Gives true when the user closes it.
pub(super) fn show(ui: &mut Ui, scene: &Scene<'_>, subject: &Subject, cluster: Option<&ClusterRow>) -> bool {
    let palette = theme::palette(ui);
    let mut close = false;
    Frame::new().inner_margin(Margin { left: 14, right: 12, top: 8, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Pod").color(palette.text_muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = ui.add(Button::new(RichText::new("Close").size(SMALL)));
                widgets::name(&button, "Close the details");
                close = button.clicked();
            });
        });
        ui.add(Label::new(RichText::new(&*subject.id.key.name).heading().strong()).truncate());
        let old = cluster.and_then(|row| row.data.as_ref()).and_then(|data| old_since(&data.pods, scene.clock));
        state_line(ui, palette, scene, subject, old.as_deref());
    });
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), (1.0, palette.border));
    ScrollArea::vertical().id_salt("pod_detail").auto_shrink(false).show(ui, |ui| {
        Frame::new().inner_margin(Margin { left: 14, right: 14, top: 10, bottom: 12 }).show(ui, |ui| {
            metadata(ui, palette, scene, subject);
            ui.add_space(14.0);
            let data = cluster.and_then(|row| row.data.as_ref());
            events::show(ui, palette, scene.clock, &events::Source { id: &subject.id, data });
        });
    });
    close
}

/// The status of the pod, and how far the panel can know it.
fn state_line(ui: &mut Ui, palette: &Palette, scene: &Scene<'_>, subject: &Subject, old: Option<&str>) {
    let status = PodStatus::of(&subject.pod);
    let current = subject.presence == Presence::Present && old.is_none();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        // The color of a state is for a state of now.
        let tone = if current { Tone::of_object(status.health) } else { Tone::Faint };
        dot(ui, tone.color(palette));
        ui.label(RichText::new(&status.text).color(tone.color(palette)));
        ui.label(RichText::new("·").color(palette.text_faint));
        dot(ui, scene.colors.color(&subject.id.cluster, palette));
        ui.label(RichText::new(subject.id.cluster.as_str()).color(palette.text_muted));
    });
    let cluster = &subject.id.cluster;
    match (subject.presence, old) {
        (Presence::Deleted, _) => {
            let text = "This pod is deleted. The panel shows its last state.";
            ui.label(RichText::new(text).color(palette.warn));
        }
        (Presence::NotLoaded, _) => {
            let text = format!("The pods of {cluster} are not loaded now. The panel shows the last state of this pod.");
            ui.label(RichText::new(text).color(palette.warn));
        }
        (Presence::Present, Some(old)) => {
            ui.horizontal(|ui| {
                ring(ui, palette);
                ui.label(RichText::new(format!("The values are {old}.")).color(palette.warn));
            });
        }
        (Presence::Present, None) => {}
    }
}

fn heading(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.label(RichText::new(title).size(11.0).color(palette.text_faint));
    ui.add_space(2.0);
}

fn metadata(ui: &mut Ui, palette: &Palette, scene: &Scene<'_>, subject: &Subject) {
    let meta = &subject.pod.metadata;
    heading(ui, palette, "METADATA");
    text_field(ui, palette, "Namespace", &subject.id.key.namespace);
    text_field(ui, palette, "Cluster", subject.id.cluster.as_str());
    text_field(ui, palette, "UID", &subject.id.uid);
    let created = meta.creation_timestamp.as_ref().map(|time| {
        let at = scene.clock.date_time(SystemTime::from(time.0));
        format!("{at} ({} ago)", age(scene.clock, time))
    });
    text_field(ui, palette, "Created", created.as_deref().unwrap_or(NONE));
    let node = subject.pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref());
    text_field(ui, palette, "Node", node.unwrap_or(NONE));
    let owners = meta.owner_references.iter().flatten().map(|owner| format!("{} {}", owner.kind, owner.name));
    let owners = owners.collect::<Vec<String>>().join(", ");
    text_field(ui, palette, "Controlled by", if owners.is_empty() { NONE } else { &owners });
    field(ui, palette, "Labels", |ui| {
        let labels = meta.labels.iter().flatten();
        chips(ui, palette, labels.map(|(key, value)| format!("{key}={value}")));
    });
    field(ui, palette, "Annotations", |ui| {
        let annotations = meta.annotations.iter().flatten();
        chips(ui, palette, annotations.map(|(key, value)| format!("{key}={}", short(value))));
    });
}

const NONE: &str = "–";

/// The start of a long value, with a mark for the part that does not show.
fn short(value: &str) -> String {
    match value.char_indices().nth(VALUE_LIMIT) {
        Some((end, _)) => format!("{}…", value.get(..end).unwrap_or_default()),
        None => value.to_owned(),
    }
}

/// One line of the metadata: the label at the left and the value at the right. A long
/// value wraps below itself and not below the label.
fn field(ui: &mut Ui, palette: &Palette, label: &str, value: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(egui::vec2(LABEL_WIDTH, 20.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(LABEL_WIDTH);
            ui.add(Label::new(RichText::new(label).color(palette.text_muted)).truncate());
        });
        ui.vertical(value);
    });
    ui.add_space(2.0);
}

fn text_field(ui: &mut Ui, palette: &Palette, label: &str, value: &str) {
    field(ui, palette, label, |ui| {
        ui.add_space(1.0);
        ui.add(Label::new(value).wrap());
    });
}

/// One chip for each text, or the mark of no value. A chip has a fixed size, so a line
/// of chips breaks between two of them.
fn chips(ui: &mut Ui, palette: &Palette, texts: impl Iterator<Item = String>) {
    let mut texts = texts.peekable();
    if texts.peek().is_none() {
        ui.add_space(1.0);
        ui.label(RichText::new(NONE).color(palette.text_faint));
        return;
    }
    // A text that is longer than the panel wraps in its chip.
    let widest = ui.available_width() - 12.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for text in texts {
            let galley = ui.painter().layout(text.clone(), theme::mono(), palette.text, widest);
            let (rect, response) = ui.allocate_exact_size(galley.size() + egui::vec2(12.0, 6.0), Sense::hover());
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &text));
            ui.painter().rect_filled(rect, 4.0, palette.bg_raised);
            ui.painter().galley(rect.min + egui::vec2(6.0, 3.0), galley, palette.text);
        }
    });
}
