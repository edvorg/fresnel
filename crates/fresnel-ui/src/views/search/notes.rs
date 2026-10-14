//! The notes of the quick search: each store that it did not read, or not in full, with
//! the reason (weakness 6 of plan section 6). Stores with one reason share a line:
//! `Not read: Secrets of fra-stage: forbidden: …`. Each kind of a line holds for each
//! cluster of the line, so a line never names a store that the search did read.

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

/// One line for each reason and each set of kinds, with the clusters that have that set
/// for that reason, in their sequence.
pub(super) fn lines(unread: &[Unread]) -> Vec<(Tone, String)> {
    // The kinds of each reason in each cluster.
    let mut of_cluster: Vec<(&Why, &ClusterId, Vec<Kind>)> = Vec::new();
    for note in unread {
        match of_cluster.iter_mut().find(|(why, cluster, _)| **why == note.why && **cluster == note.cluster) {
            Some((.., kinds)) => kinds.extend(note.kind),
            None => of_cluster.push((&note.why, &note.cluster, note.kind.into_iter().collect())),
        }
    }
    let mut groups: Vec<(&Why, Vec<Kind>, Vec<&ClusterId>)> = Vec::new();
    for (why, cluster, mut kinds) in of_cluster {
        kinds.sort_unstable();
        kinds.dedup();
        match groups.iter_mut().find(|(other, other_kinds, _)| *other == why && *other_kinds == kinds) {
            Some((.., clusters)) => clusters.push(cluster),
            None => groups.push((why, kinds, vec![cluster])),
        }
    }
    groups.into_iter().map(|(why, kinds, clusters)| line(why, &kinds, &clusters)).collect()
}

fn line(why: &Why, kinds: &[Kind], clusters: &[&ClusterId]) -> (Tone, String) {
    let clusters = clusters.iter().map(|cluster| cluster.as_str()).collect::<Vec<_>>().join(", ");
    let titles = kinds.iter().map(|kind| kind.info().title).collect::<Vec<_>>().join(", ");
    let place = if titles.is_empty() { clusters } else { format!("{titles} of {clusters}") };
    let (tone, start) = match why {
        Why::NotSelected => (Tone::Warn, "Not read"),
        Why::NoStores(_) | Why::Refused(_) => (Tone::Error, "Not read"),
        Why::NoStore => (Tone::Muted, "Not read"),
        Why::Incomplete(_) => (Tone::Info, "Not complete"),
    };
    (tone, format!("{start}: {place}: {why}"))
}

#[cfg(test)]
mod tests {
    use fresnel_core::store::{StoreState, Wait};

    use super::*;

    fn unread(cluster: &str, kind: Option<Kind>, why: &Why) -> Unread {
        Unread { cluster: ClusterId::new(cluster), kind, why: why.clone() }
    }

    /// Two clusters wait for list slots for other kinds. One line for both would name a
    /// kind of one cluster for the other one, whose store the search did read.
    #[test]
    fn each_kind_of_a_line_holds_for_each_cluster_of_the_line() {
        let waits = Why::Incomplete(StoreState::Waiting(Wait::ListSlot));
        let no_server = Why::NoStores("the kubeconfig entry has no server".to_owned());
        let notes = [
            unread("ams", Some(Kind::Secret), &waits),
            unread("fra", Some(Kind::ConfigMap), &waits),
            unread("ams", Some(Kind::ConfigMap), &waits),
            unread("nyc", Some(Kind::ConfigMap), &waits),
            unread("nyc", Some(Kind::Secret), &waits),
            unread("lon", None, &no_server),
            unread("sgp", None, &Why::NotSelected),
        ];
        let texts: Vec<String> = lines(&notes).into_iter().map(|(_, text)| text).collect();
        let part = "its first list is not complete (waits for a list slot); the search read a part";
        assert_eq!(
            texts,
            [
                format!("Not complete: ConfigMaps, Secrets of ams, nyc: {part}"),
                format!("Not complete: ConfigMaps of fra: {part}"),
                "Not read: lon: no store: the kubeconfig entry has no server".to_owned(),
                "Not read: sgp: the cluster is not selected".to_owned(),
            ]
        );
    }
}
