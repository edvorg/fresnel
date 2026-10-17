//! The confirm dialog (plan section 11.2): it lists each target as cluster, namespace, kind
//! and name, with the color of its cluster, takes the input of the action (a count, a name,
//! a propagation policy, a revision), and tells the warnings of `research/08`.
//!
//! The dialog decides each target in each frame from its store and the permissions: a target
//! that goes away, that a new object of its name replaces, that changes its state or that
//! becomes forbidden while the dialog is open shows its reason, and the button is off. So is
//! a count of targets over the jobs that the app can start now.

use egui::{Align, Grid, Layout, RichText, ScrollArea, TextEdit, Ui};
use fresnel_core::kinds::{Action, Kind};
use fresnel_core::ops::{self, Operation, Order, Propagation, Target};

use super::dialog::{Close, title, warning};
use super::{Found, Gate, Permits, Subject, TypedName, not_of_kind, operation, over_room};
use crate::format::{self, short};
use crate::shell::Scene;
use crate::theme::{self, ROW_HEIGHT};
use crate::views::detail::fields::VALUE_LIMIT;
use crate::widgets::{self, dot};

/// The longest name of a Job: the server writes it into a label value.
const NAME_LIMIT: usize = 63;
/// The height of the list of the targets before it scrolls.
const LIST_HEIGHT: f32 = 220.0;

/// What the user gives to the action.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Input {
    None,
    /// The count of pods, as the user types it.
    Count(String),
    /// The name of the Job of a trigger.
    Name(String),
    Propagation(Propagation),
    /// The revision of a rollback, and the one that the user chose in the list of the
    /// revisions. 0 is the revision before the current one.
    Revision {
        to: i64,
        offered: Option<i64>,
    },
}

pub(crate) struct Confirm {
    action: Action,
    subjects: Vec<Subject>,
    input: Input,
    permits: Permits,
    typed: TypedName,
}

impl Confirm {
    /// The dialog of an action on its objects. `revision` is the choice of a rollback.
    pub(crate) fn new(action: Action, subjects: Vec<Subject>, revision: Option<i64>) -> Self {
        let input = match action {
            Action::Scale => {
                let mut counts = subjects.iter().map(|subject| subject.facts.replicas);
                let first = counts.next().flatten();
                let same = counts.all(|count| count == first);
                Input::Count(first.filter(|_| same).map(|count| count.to_string()).unwrap_or_default())
            }
            Action::Trigger => {
                let name = subjects.first().map(|subject| ops::trigger_name(&subject.target.key.name));
                Input::Name(name.unwrap_or_default())
            }
            Action::Delete => Input::Propagation(Propagation::Background),
            Action::Rollback => Input::Revision { to: revision.unwrap_or(0), offered: revision },
            _ => Input::None,
        };
        Self { action, subjects, input, permits: Permits::default(), typed: TypedName::default() }
    }

    /// The operation of the input, or why the input does not give one.
    fn operation(&self) -> Result<Operation, String> {
        match &self.input {
            Input::Count(text) => match text.trim().parse::<i32>() {
                Ok(count) if count >= 0 => Ok(Operation::Scale(count)),
                _ => Err("The count of pods is a number of 0 or more.".to_owned()),
            },
            Input::Name(name) => valid_name(name.trim()).map(|()| Operation::Trigger(name.trim().to_owned())),
            Input::Propagation(propagation) => Ok(Operation::Delete(*propagation)),
            Input::Revision { to, .. } => Ok(Operation::Rollback(*to)),
            Input::None => operation(self.action).ok_or_else(|| not_of_kind(self.action)),
        }
    }

    /// Draws the dialog. `look` finds each target in the store of its cluster.
    pub(super) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>, look: &dyn Fn(&Target) -> Found) -> Close {
        let palette = theme::palette(ui);
        title(ui, &self.title());
        ui.label(RichText::new("The action acts on each of these objects:").color(palette.text_muted));
        ui.add_space(4.0);
        let gates = self.targets(ui, scene, look);
        ui.add_space(8.0);
        self.input(ui);
        for text in self.warnings() {
            warning(ui, palette, &text);
        }
        let clusters = self.subjects.iter().map(|subject| &subject.target.cluster);
        let typed = self.typed.show(ui, palette, scene.view, clusters);
        ui.add_space(8.0);
        let operation = self.operation();
        let refused = gates.iter().find_map(|(subject, gate)| match gate {
            Gate::Closed(reason) => Some(format!("{}: {reason}", subject)),
            Gate::Open | Gate::Unknown(_) => None,
        });
        let full = || over_room(self.subjects.len(), scene.view.jobs.room());
        let reason = refused.or_else(full).or_else(|| operation.as_ref().err().cloned()).or(typed);
        if let Some(reason) = &reason {
            ui.add(egui::Label::new(RichText::new(reason).color(palette.error)).wrap());
        }
        let mut close = Close::Open;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let confirm = ui.add_enabled(reason.is_none(), egui::Button::new(self.action.label()));
            widgets::name(&confirm, &format!("Confirm {}", self.action.label()));
            if confirm.clicked()
                && let Ok(operation) = &operation
            {
                close = self.run(operation);
            }
            if ui.button("Cancel").clicked() {
                close = Close::Cancelled;
            }
        });
        close
    }

    /// "Scale the Deployment web", "Restart 3 Deployments".
    fn title(&self) -> String {
        let label = self.action.label();
        match self.subjects.as_slice() {
            [one] => format!("{label}: the {} {}", one.target.kind.info().name, one.target.key.name),
            many => {
                let kind = many.first().map_or(Kind::Pod, |subject| subject.target.kind);
                format!("{label}: {}", format::count(many.len() as u64, kind.info().name))
            }
        }
    }

    /// The list of the targets, with the gate of each one for this frame. A target that its
    /// store has gets its facts of now.
    fn targets(&mut self, ui: &mut Ui, scene: &mut Scene<'_>, look: &dyn Fn(&Target) -> Found) -> Vec<(String, Gate)> {
        let palette = theme::palette(ui);
        let (action, permits) = (self.action, &mut self.permits);
        let gates: Vec<(String, Gate)> = self
            .subjects
            .iter_mut()
            .map(|subject| {
                let gate = match look(&subject.target) {
                    Found::Gone(reason) => Gate::Closed(reason),
                    found => {
                        if let Found::Here(facts) = found {
                            subject.facts = facts;
                        }
                        let decide = || permits.decide(scene.view, &subject.target, action, scene.commands);
                        Gate::in_cluster(scene.view, &subject.target.cluster, (action, &subject.facts), decide)
                    }
                };
                (subject.text(), gate)
            })
            .collect();
        let rows = self.subjects.len();
        ScrollArea::vertical().max_height(LIST_HEIGHT).auto_shrink([false, true]).show_rows(
            ui,
            ROW_HEIGHT,
            rows,
            |ui, range| {
                Grid::new("targets").num_columns(5).spacing([14.0, 4.0]).min_row_height(ROW_HEIGHT - 4.0).show(
                    ui,
                    |ui| {
                        for index in range {
                            let (Some(subject), Some((_, gate))) = (self.subjects.get(index), gates.get(index)) else {
                                continue;
                            };
                            let target = &subject.target;
                            ui.horizontal(|ui| {
                                dot(ui, scene.colors.color(&target.cluster, palette));
                                ui.label(target.cluster.as_str());
                            });
                            let namespace = if target.key.namespace.is_empty() { "–" } else { &target.key.namespace };
                            ui.label(RichText::new(short(namespace, VALUE_LIMIT)).color(palette.text_muted));
                            ui.label(RichText::new(target.kind.info().name).color(palette.text_muted));
                            ui.label(RichText::new(short(&target.key.name, VALUE_LIMIT)).strong());
                            match gate {
                                Gate::Open => ui.label(""),
                                Gate::Unknown(_) => {
                                    let label =
                                        ui.label(RichText::new("permission not known").color(palette.text_faint));
                                    label.on_hover_text(gate.hint().unwrap_or_default())
                                }
                                Gate::Closed(reason) => {
                                    ui.label(RichText::new(short(reason, VALUE_LIMIT)).color(palette.error))
                                }
                            };
                            ui.end_row();
                        }
                    },
                );
            },
        );
        gates
    }

    /// The input of the action.
    fn input(&mut self, ui: &mut Ui) {
        match &mut self.input {
            Input::None => {}
            Input::Count(text) => {
                ui.horizontal(|ui| {
                    ui.label("Count of pods");
                    let field = ui.add(TextEdit::singleline(text).desired_width(80.0));
                    widgets::name(&field, "Count of pods");
                });
            }
            Input::Name(name) => {
                ui.horizontal(|ui| {
                    ui.label("Name of the Job");
                    let field = ui.add(TextEdit::singleline(name).desired_width(360.0).char_limit(NAME_LIMIT));
                    widgets::name(&field, "Name of the Job");
                });
            }
            Input::Propagation(propagation) => {
                ui.radio_value(propagation, Propagation::Background, "Delete the dependents in the background");
                ui.radio_value(propagation, Propagation::Foreground, "Delete the dependents first, then the object");
                ui.radio_value(propagation, Propagation::Orphan, "Keep the dependents, with no owner");
            }
            Input::Revision { to, offered } => {
                ui.radio_value(to, 0, "Roll back to the revision before the current one");
                if let Some(offered) = *offered {
                    ui.radio_value(to, offered, format!("Roll back to revision {offered}"));
                }
            }
        }
    }

    /// The warnings of the action for its objects (`research/08` sections 4 and 5).
    fn warnings(&self) -> Vec<String> {
        let any = |test: fn(&Subject) -> bool| self.subjects.iter().any(test);
        let mut texts = Vec::new();
        match (self.action, &self.input) {
            (Action::Restart, _) => texts.push("Each pod starts again, in the sequence of the update strategy.".to_owned()),
            (Action::Rollback, _) if any(|subject| subject.facts.on_delete) => texts.push(
                "The update strategy is OnDelete: the pods keep the old template until someone deletes them.".to_owned(),
            ),
            (Action::Delete, Input::Propagation(Propagation::Orphan)) => {
                texts.push("The dependents stay and run on, with no owner.".to_owned());
            }
            (Action::ForceDelete, _) => {
                texts.push(
                    "The server removes the object at once and does not wait for the kubelet: the process can run on the node with no end."
                        .to_owned(),
                );
                if any(|subject| subject.facts.stateful_pod) {
                    texts.push("A pod of a StatefulSet: a new pod with the same identity can start while the old one runs.".to_owned());
                }
            }
            (Action::Evict, _) => texts.push("The eviction obeys each PodDisruptionBudget: a budget can refuse it.".to_owned()),
            (Action::Download, _) => texts.push(
                "The file in Downloads has the value of each key of the Secret, as base64: each program of your user can read it."
                    .to_owned(),
            ),
            (Action::Cordon, _) => texts.push("The scheduler puts no new pod on the node. Its pods go on to run.".to_owned()),
            (Action::Uncordon, _) => texts.push("The scheduler can put new pods on the node again.".to_owned()),
            (Action::RunAgain, _) => {
                texts.push("The Job goes away with its pods, and a new Job with the same manifest starts.".to_owned());
                if any(|subject| subject.facts.running) {
                    texts.push("The Job did not end: its pods stop now.".to_owned());
                }
            }
            (Action::Suspend, _) => texts.push(
                "A suspended Job stops its active pods; a suspended CronJob starts no new Job, and its Jobs that run go on."
                    .to_owned(),
            ),
            (Action::Trigger, _) => {
                let policy = self.subjects.first().and_then(|subject| subject.facts.concurrency.as_deref());
                match policy {
                    Some("Forbid") => texts.push(
                        "The concurrency policy is Forbid: while this Job runs, the next scheduled Job does not start."
                            .to_owned(),
                    ),
                    Some("Replace") => texts
                        .push("The concurrency policy is Replace: the next scheduled Job replaces this one.".to_owned()),
                    _ => {}
                }
            }
            _ => {}
        }
        texts
    }

    /// The orders of the operation: one for each object, so each one gets its own job.
    fn run(&self, operation: &Operation) -> Close {
        let targets = self.subjects.iter().map(|subject| subject.target.clone());
        match Order::bulk(targets, operation) {
            Ok(orders) => {
                let note = match self.subjects.as_slice() {
                    [one] => {
                        format!("Started: {} of {}. The Operations tab shows the job.", operation.label(), one.text())
                    }
                    many => {
                        format!("Started {} jobs: {}. The Operations tab shows them.", many.len(), operation.label())
                    }
                };
                Close::Run { orders, note }
            }
            Err(error) => Close::Run { orders: Vec::new(), note: error.to_string() },
        }
    }
}

/// The rule of the server for the name of a Job: a DNS label, as it writes the name into
/// a label value.
fn valid_name(name: &str) -> Result<(), String> {
    let first_and_last = name.chars().next().zip(name.chars().last());
    let alphanumeric = |char: char| char.is_ascii_lowercase() || char.is_ascii_digit();
    let fits = name.len() <= NAME_LIMIT
        && first_and_last.is_some_and(|(first, last)| alphanumeric(first) && alphanumeric(last))
        && name.chars().all(|char| alphanumeric(char) || char == '-');
    if fits {
        Ok(())
    } else {
        Err(format!(
            "The name of a Job has 1 to {NAME_LIMIT} small letters, digits and '-', and starts and ends with a letter or a digit."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_of_a_job_follows_the_rule_of_a_label() {
        assert_eq!(valid_name("backup-manual-a1b2c"), Ok(()));
        for bad in ["", "-a", "a-", "Backup", "a_b", &"a".repeat(64)] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
    }
}
