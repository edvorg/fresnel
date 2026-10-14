//! The notes of the quick search: each store that it did not read, or not in full, with
//! the reason (weakness 6 of plan section 6). Stores with one reason share a line:
//! `Not read: Secrets of fra-stage: forbidden: …`.

use egui::{Label, RichText, ScrollArea, Ui};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::search::{Unread, Why};

use crate::theme::{Palette, ROW_HEIGHT};
use crate::widgets::tone::Tone;

/// The most notes that show with no scroll.
const LINES: f32 = 5.0;

/// Draws the notes. A search that read each store in full has none.
pub(super) fn show(ui: &mut Ui, palette: &Palette, unread: &[Unread]) {
    let lines = lines(unread);
    if lines.is_empty() {
        return;
    }
    ui.separator();
    let list = ScrollArea::vertical().id_salt("quick search notes").max_height(LINES * ROW_HEIGHT);
    list.auto_shrink([false, true]).show(ui, |ui| {
        for (tone, text) in &lines {
            ui.add(Label::new(RichText::new(text).color(tone.color(palette))).truncate());
        }
    });
}

/// One line for each reason, with its kinds and its clusters in their sequence.
pub(super) fn lines(unread: &[Unread]) -> Vec<(Tone, String)> {
    let mut groups: Vec<(&Why, Vec<Kind>, Vec<&ClusterId>)> = Vec::new();
    for note in unread {
        let index = match groups.iter().position(|(why, ..)| **why == note.why) {
            Some(index) => index,
            None => {
                groups.push((&note.why, Vec::new(), Vec::new()));
                groups.len() - 1
            }
        };
        let Some((_, kinds, clusters)) = groups.get_mut(index) else {
            continue;
        };
        kinds.extend(note.kind.filter(|kind| !kinds.contains(kind)));
        if !clusters.contains(&&note.cluster) {
            clusters.push(&note.cluster);
        }
    }
    groups.into_iter().map(|(why, mut kinds, clusters)| line(why, &mut kinds, &clusters)).collect()
}

fn line(why: &Why, kinds: &mut [Kind], clusters: &[&ClusterId]) -> (Tone, String) {
    kinds.sort_unstable();
    let clusters = clusters.iter().map(|cluster| cluster.as_str()).collect::<Vec<_>>().join(", ");
    let titles = kinds.iter().map(|kind| kind.info().title).collect::<Vec<_>>().join(", ");
    let place = if titles.is_empty() { clusters } else { format!("{titles} of {clusters}") };
    let (tone, start) = match why {
        Why::NoStores(_) | Why::Refused(_) => (Tone::Error, "Not read"),
        Why::NoStore => (Tone::Muted, "Not read"),
        Why::Incomplete(_) => (Tone::Info, "Not complete"),
    };
    (tone, format!("{start}: {place}: {why}"))
}
