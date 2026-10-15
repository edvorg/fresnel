//! The drain dialog (plan section 10): the options of `kubectl drain` and each pod of the node
//! with its verdict for these options: evict, keep or block. A pod that blocks turns the
//! button off, with the option that removes its reason.
//!
//! The dialog reads the pods of the node from the reference index and the DaemonSets from
//! their store, in each frame. The job reads the list of the server again after its cordon,
//! and stops before its first eviction when a pod blocks then.

use std::time::Duration;

use egui::{Align, Layout, RichText, ScrollArea, TextEdit, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::kinds::{Action, Kind};
use fresnel_core::ops::drain::{self, DrainOptions, Found as DaemonSet, Verdict};
use fresnel_core::ops::{Operation, Order, Target};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::apps::v1::DaemonSet as DaemonSetObject;

use super::dialog::{Close, title, warning};
use super::{Found, Gate, Permits, Subject, over_room};
use crate::shell::Scene;
use crate::theme::{self, ROW_HEIGHT};
use crate::views::operations::drain::verdict as verdict_cell;
use crate::widgets::mini_table::{self, Column, Text};
use crate::widgets::{self, dot};

const COLUMNS: &[Column] = &[Column::fill("Pod"), Column::fill("Verdict")];
/// The height of the list of the pods before it scrolls.
const LIST_HEIGHT: f32 = 240.0;

pub(crate) struct DrainDialog {
    subject: Subject,
    options: DrainOptions,
    /// The grace period and the time limit, in seconds, as the user types them.
    grace: String,
    timeout: String,
    permits: Permits,
}

/// The pods of the node with their verdicts, and a note when the list can miss some.
struct Pods {
    verdicts: Vec<(ObjectKey, Verdict)>,
    note: Option<&'static str>,
}

impl DrainDialog {
    pub(crate) fn new(subject: Subject) -> Self {
        let (grace, timeout) = (String::new(), String::new());
        Self { subject, options: DrainOptions::default(), grace, timeout, permits: Permits::default() }
    }

    /// The options of the input, or why the input gives none.
    fn options(&self) -> Result<DrainOptions, String> {
        let seconds = |text: &str| text.trim().parse::<u32>().ok().filter(|_| !text.trim().is_empty());
        let grace = match self.grace.trim() {
            "" => None,
            text => Some(seconds(text).ok_or("The grace period is a count of seconds, or empty.")?),
        };
        let timeout = match self.timeout.trim() {
            "" | "0" => None,
            text => Some(Duration::from_secs(
                seconds(text).ok_or("The time limit is a count of seconds, or empty.")?.into(),
            )),
        };
        Ok(DrainOptions { grace_period: grace, timeout, ..self.options })
    }

    pub(super) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>, look: &dyn Fn(&Target) -> Found) -> Close {
        let palette = theme::palette(ui);
        let target = self.subject.target.clone();
        title(ui, &format!("Drain: the Node {}", target.key.name));
        ui.horizontal(|ui| {
            dot(ui, scene.colors.color(&target.cluster, palette));
            ui.label(target.cluster.as_str());
        });
        let text = "The drain cordons the node and evicts each pod that the list marks \"evict\", as kubectl drain. \
            A PodDisruptionBudget can hold an eviction; the Operations tab shows each pod.";
        ui.add(egui::Label::new(RichText::new(text).color(palette.text_muted)).wrap());
        ui.add_space(6.0);
        self.inputs(ui);
        let options = self.options();
        let pods = pods(scene.view, &self.subject, &options.clone().unwrap_or(self.options));
        ui.add_space(6.0);
        let blocks = match &pods {
            Ok(pods) => list(ui, pods),
            Err(reason) => {
                warning(ui, palette, reason);
                0
            }
        };
        let gate = match look(&target) {
            Found::Gone(reason) => Gate::Closed(reason),
            found => {
                if let Found::Here(facts) = found {
                    self.subject.facts = facts;
                }
                let decide = || self.permits.decide(scene.view, &target, Action::Drain, scene.commands);
                Gate::of(Action::Drain, &self.subject.facts, decide)
            }
        };
        let reason = match (&gate, &options) {
            (Gate::Closed(reason), _) => Some(reason.clone()),
            (_, Err(reason)) => Some(reason.clone()),
            _ if blocks > 0 => Some(format!(
                "{} the drain: turn on the option that its line names.",
                if blocks == 1 { "1 pod blocks".to_owned() } else { format!("{blocks} pods block") }
            )),
            _ => over_room(1, scene.view.jobs.room()),
        };
        // The line stays when it is empty: a modal dialog sits in the middle, so a change of
        // its height would move each checkbox away from the pointer.
        ui.add(egui::Label::new(RichText::new(reason.as_deref().unwrap_or_default()).color(palette.error)).truncate());
        let mut close = Close::Open;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let confirm = ui.add_enabled(reason.is_none(), egui::Button::new("Drain"));
            widgets::name(&confirm, "Confirm Drain");
            if confirm.clicked()
                && let Ok(options) = options
            {
                close = run(&self.subject, options);
            }
            if ui.button("Cancel").clicked() {
                close = Close::Cancelled;
            }
        });
        close
    }

    fn inputs(&mut self, ui: &mut Ui) {
        let options = &mut self.options;
        ui.checkbox(&mut options.ignore_daemon_sets, "Ignore DaemonSets: their pods stay on the node");
        ui.checkbox(&mut options.delete_empty_dir_data, "Delete emptyDir data: evict a pod with an emptyDir volume");
        ui.checkbox(&mut options.force, "Force: evict a pod with no controller, which nothing starts again");
        for (label, text, hint) in
            [("Grace period (s)", &mut self.grace, "each pod's own"), ("Time limit (s)", &mut self.timeout, "no limit")]
        {
            ui.horizontal(|ui| {
                ui.label(label);
                let field = ui.add(TextEdit::singleline(text).desired_width(120.0).hint_text(hint));
                widgets::name(&field, label);
            });
        }
    }
}

/// The pods of the node with their verdicts for these options, or why the dialog has none.
fn pods(view: &CoreView, node: &Subject, options: &DrainOptions) -> Result<Pods, String> {
    let target = &node.target;
    let row = view.clusters.iter().find(|row| row.id == target.cluster);
    let data = row.and_then(|row| row.data.as_ref()).ok_or_else(|| format!("{} is not connected.", target.cluster))?;
    let answer = data.references.used_by(Kind::Node, &target.key);
    let daemon_sets = data.snapshot::<DaemonSetObject>();
    let found = |namespace: &str, name: &str| match &daemon_sets {
        Some(sets) if sets.items.get(&ObjectKey::new(namespace, name)).is_some() => DaemonSet::Here,
        Some(sets) if sets.listed => DaemonSet::Missing,
        _ => DaemonSet::Unknown("the DaemonSets are not loaded yet".to_owned()),
    };
    let verdicts = answer
        .found
        .iter()
        .filter_map(|key| {
            let pod = data.pods.items.get(key)?;
            Some((key.clone(), drain::verdict(pod, options, |name| found(&key.namespace, name))))
        })
        .collect();
    let note = (!answer.basis.is_loaded() || !data.pods.listed)
        .then_some("The pods of the node are not loaded yet. The drain reads its own list from the server.");
    Ok(Pods { verdicts, note })
}

/// Draws the list of the pods. Gives the count of the pods that block.
fn list(ui: &mut Ui, pods: &Pods) -> usize {
    let palette = theme::palette(ui);
    let count = |test: fn(&Verdict) -> bool| pods.verdicts.iter().filter(|(_, verdict)| test(verdict)).count();
    let evict = count(|verdict| matches!(verdict, Verdict::Evict(_)));
    let keep = count(|verdict| matches!(verdict, Verdict::Keep(_)));
    let blocks = count(|verdict| matches!(verdict, Verdict::Block(_)));
    let summary = format!("{} pods: {evict} to evict, {keep} to keep, {blocks} block", pods.verdicts.len());
    ui.label(RichText::new(summary).color(palette.text_muted));
    // The note has its line also when it is empty, and the list its height: see `show`.
    ui.add(egui::Label::new(RichText::new(pods.note.unwrap_or_default()).color(palette.warn)).truncate());
    mini_table::header(ui, "pods of the drain dialog", COLUMNS);
    let scroll = ScrollArea::vertical().min_scrolled_height(LIST_HEIGHT).max_height(LIST_HEIGHT);
    scroll.auto_shrink(false).show_rows(ui, ROW_HEIGHT, pods.verdicts.len(), |ui, range| {
        for (key, verdict) in pods.verdicts.get(range).unwrap_or_default() {
            let name = Text::new(format!("{}/{}", key.namespace, key.name));
            mini_table::row(ui, ("drain dialog", key), COLUMNS, &[name, verdict_cell(verdict)]);
        }
    });
    blocks
}

/// The order of the drain with these options.
fn run(node: &Subject, options: DrainOptions) -> Close {
    match Order::new(node.target.clone(), Operation::Drain(options)) {
        Ok(order) => {
            let note =
                format!("Started: {} of {}. The Operations tab shows each pod.", order.operation.label(), node.text());
            Close::Run { orders: vec![order], note }
        }
        Err(error) => Close::Run { orders: Vec::new(), note: error.to_string() },
    }
}
