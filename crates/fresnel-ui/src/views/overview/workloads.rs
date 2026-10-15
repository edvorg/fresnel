//! The table of the Workloads overview: for each workload kind, a line for each selected
//! cluster with its objects by health (`research/01` section 3.4.2, `research/02` section
//! 5.1). The title of a kind is a link to its table, and a count of objects with a warning
//! or a failure is a link to the table with the "problems" filter, whose rows are the
//! objects of the count. A line of a store that is not live has the mark of that store.

use egui::{Grid, RichText, Ui};
use fresnel_core::app::ClusterRow;
use fresnel_core::kinds::{Kind, ObjectHealth, Sidebar};

use super::Jump;
use super::count::{Counted, Tally};
use crate::format;
use crate::shell::Scene;
use crate::theme::{self, Palette};
use crate::views::detail::fields::NONE;
use crate::views::objects::count_mark;
use crate::widgets::table::cells::ring;
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The columns of the counts, in the sequence of the health values.
const HEALTH: [(ObjectHealth, &str); 4] = [
    (ObjectHealth::Healthy, "Healthy"),
    (ObjectHealth::Progressing, "Progressing"),
    (ObjectHealth::Warning, "Warning"),
    (ObjectHealth::Failed, "Failed"),
];

pub(super) fn show(ui: &mut Ui, scene: &Scene<'_>, counted: &[Counted]) -> Option<Jump> {
    let palette = theme::palette(ui);
    let selected: Vec<&ClusterRow> = scene.view.clusters.iter().filter(|row| row.selected).collect();
    if selected.is_empty() {
        ui.label(RichText::new("No cluster is selected. Select a cluster in the top bar.").color(palette.text_muted));
        return None;
    }
    let mut jump = None;
    Grid::new("workloads").num_columns(8).spacing([18.0, 6.0]).striped(true).show(ui, |ui| {
        for title in ["Kind", "Cluster", "Objects"].into_iter().chain(HEALTH.map(|(_, title)| title)) {
            ui.label(RichText::new(title).size(11.0).color(palette.text_faint));
        }
        ui.end_row();
        for kind in Sidebar::Workloads.kinds() {
            for (place, row) in selected.iter().enumerate() {
                let title = kind.info().title;
                if place == 0 && ui.link(title).clicked() {
                    jump = Some(Jump::Table { kind, problems: false });
                } else if place > 0 {
                    ui.label("");
                }
                jump = line(ui, palette, scene, row, kind, counted).or(jump.take());
                ui.end_row();
            }
        }
    });
    jump
}

/// The line of one kind in one cluster: its cluster, its counts and the mark of its store.
fn line(
    ui: &mut Ui,
    palette: &Palette,
    scene: &Scene<'_>,
    row: &ClusterRow,
    kind: Kind,
    counted: &[Counted],
) -> Option<Jump> {
    ui.horizontal(|ui| {
        dot(ui, scene.colors.color(&row.id, palette));
        ui.label(row.id.as_str());
    });
    let counts = counted.iter().find(|counts| counts.cluster == row.id);
    let tally = counts.and_then(|counts| tally(counts, kind));
    let mut jump = None;
    match tally {
        Some(tally) => {
            let total = format::thousands(tally.total().into());
            spoken(&ui.label(&total), &format!("{} of {}: {total} objects", kind.info().title, row.id));
            for (health, title) in HEALTH {
                jump = cell(ui, palette, row, kind, tally.of(health), (health, title)).or(jump.take());
            }
        }
        None => (0..=HEALTH.len()).for_each(|_| {
            ui.label(RichText::new(NONE).color(palette.text_faint));
        }),
    }
    let status = row.data.as_ref().and_then(|data| match kind {
        Kind::Pod => Some(data.pods.status(Kind::Pod)),
        _ => data.objects.status(kind).cloned(),
    });
    let mark = match (&row.data, &status) {
        (None, _) => Some(row.reason().unwrap_or_else(|| "the connect starts".to_owned())),
        (Some(_), Some(status)) => count_mark(status, scene.clock),
        (Some(_), None) => Some("not loaded: the store starts".to_owned()),
    };
    if let Some(mark) = mark {
        ui.horizontal(|ui| {
            ring(ui, palette);
            ui.label(RichText::new(mark).color(palette.warn));
        });
    }
    jump
}

/// The objects of a kind by health in the counts of a cluster. `None` for a kind whose
/// store the core does not hold yet.
fn tally(counts: &Counted, kind: Kind) -> Option<Tally> {
    match kind {
        Kind::Pod => Some(counts.pods),
        _ => counts.kinds.iter().find(|(of, _)| *of == kind).map(|(_, tally)| *tally),
    }
}

/// One count. A count of problems that is not zero is a link to the table of its kind with
/// the "problems" filter. Its name for a screen reader tells its kind and its cluster: a
/// number of a grid alone tells nothing.
fn cell(
    ui: &mut Ui,
    palette: &Palette,
    row: &ClusterRow,
    kind: Kind,
    count: u32,
    (health, title): (ObjectHealth, &str),
) -> Option<Jump> {
    let text = format::thousands(count.into());
    let name = format!("{} of {}: {text} {}", kind.info().title, row.id, title.to_lowercase());
    if count == 0 || !health.is_problem() {
        let tone = if count == 0 { Tone::Faint } else { Tone::Normal };
        spoken(&ui.label(RichText::new(text).color(tone.color(palette))), &name);
        return None;
    }
    let link = ui.link(RichText::new(text).color(Tone::of_object(health).color(palette)));
    widgets::name(&link, &name);
    link.clicked().then_some(Jump::Table { kind, problems: true })
}

/// Gives a text of the grid the words of a screen reader, which reads the value of a text.
fn spoken(text: &egui::Response, words: &str) {
    text.ctx.accesskit_node_builder(text.id, |node| node.set_value(words));
}
