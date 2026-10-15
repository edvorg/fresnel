//! The list of the quick search: the commands of the command table that match the text,
//! then the hits. Each hit shows its cluster, kind, namespace and name, and the mark of a
//! store that is not live.

use std::collections::BTreeSet;

use egui::{Align2, Color32, FontId, Rect, ScrollArea, Sense, TextStyle, Ui, WidgetInfo, WidgetType};
use fresnel_core::commands::{self, CommandId};
use fresnel_core::search::{Found, Hit};

use super::Choice;
use crate::shell::Scene;
use crate::theme::{self, Palette, ROW_HEIGHT, SMALL};
use crate::views::detail::actions;
use crate::views::objects::{Link, old_text};
use crate::widgets;

/// The height of the list. More rows scroll.
const HEIGHT: f32 = 360.0;
/// The left of the column of the cluster, right of the dot of the cluster.
const CLUSTER: f32 = 22.0;
/// The most width of the texts of the cluster and of the namespace, as their fixed columns
/// had: a longer one is cut. A kind is never cut: its names are of the app, and the longest
/// has some 175 points.
const CLUSTER_MOST: f32 = 140.0;
const NAMESPACE_MOST: f32 = 142.0;
/// The room between two columns.
const GAP: f32 = 16.0;
/// The room at the right end of a row.
const RIGHT: f32 = 8.0;
/// The room of the ring of the mark of an old store, left of its words.
const RING: f32 = 12.0;
/// The words of the first column of a command.
const COMMAND: &str = "Command";

/// One row of the list.
pub(super) enum Row {
    Command { id: CommandId, label: &'static str, key: Option<String> },
    Hit(Hit),
}

impl Row {
    /// What the row does: the identity of the row too.
    pub(super) fn choice(&self) -> Choice {
        match self {
            Self::Command { id, .. } => Choice::Run(*id),
            Self::Hit(hit) => Choice::Open(Link { cluster: hit.cluster.clone(), kind: hit.kind, key: hit.key.clone() }),
        }
    }

    /// True for the row of this choice. It copies no text, so the search for the row of
    /// the cursor in each frame is cheap.
    pub(super) fn is(&self, choice: &Choice) -> bool {
        match (self, choice) {
            (Self::Command { id, .. }, Choice::Run(chosen)) => id == chosen,
            (Self::Hit(hit), Choice::Open(link)) => {
                (&hit.cluster, hit.kind, &hit.key) == (&link.cluster, link.kind, &link.key)
            }
            (Self::Command { .. }, Choice::Open(_)) | (Self::Hit(_), Choice::Run(_)) => false,
        }
    }
}

/// The rows for a text: each command that this version runs and whose label has each word
/// of the text, then the hits of the result. `key` gives the text of the key of a command.
pub(super) fn rows(found: Option<&Found>, text: &str, key: impl Fn(CommandId) -> Option<String>) -> Vec<Row> {
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    // A move of the focus is for the keys: the search itself has the focus.
    let runs = |id: CommandId| match id {
        CommandId::QuickSearch | CommandId::Focus(_) | CommandId::NextArea | CommandId::PreviousArea => false,
        CommandId::Back | CommandId::Forward | CommandId::Filter | CommandId::NewSecret => true,
        CommandId::Settings | CommandId::Help => true,
        CommandId::Object(action) => actions::can_run(action),
    };
    let matches = |label: &str| !words.is_empty() && words.iter().all(|word| label.to_lowercase().contains(word));
    let commands = commands::TABLE.iter().filter(|entry| runs(entry.id) && matches(entry.label));
    let mut rows: Vec<Row> =
        commands.map(|entry| Row::Command { id: entry.id, label: entry.label, key: key(entry.id) }).collect();
    rows.extend(found.into_iter().flat_map(|found| found.hits.iter().cloned()).map(Row::Hit));
    rows
}

/// Draws the rows. `moved` is true when a key moved the cursor: the list then scrolls to
/// it. Gives the row of a click.
pub(super) fn show(ui: &mut Ui, scene: &Scene<'_>, rows: &[Row], cursor: usize, moved: bool) -> Option<usize> {
    let palette = theme::palette(ui);
    let places = Places::of(ui, rows);
    let mut clicked = None;
    let list = ScrollArea::vertical().id_salt("quick search list").max_height(HEIGHT).auto_shrink([false, true]);
    list.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
        for index in range {
            let Some(row) = rows.get(index) else {
                continue;
            };
            let response = draw(ui, (palette, scene), places, row, index == cursor);
            if moved && index == cursor {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                clicked = Some(index);
            }
        }
    });
    clicked
}

/// The left of each column of a hit in a row, after the cluster: the kind, the namespace and
/// the name. A command has its label in the place of the kind.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Places {
    kind: f32,
    namespace: f32,
    name: f32,
}

impl Places {
    /// The places for these rows. The cluster, the kind and the namespace are as wide as
    /// their widest texts in the rows, so the name gets the room that short texts leave.
    fn of(ui: &Ui, rows: &[Row]) -> Self {
        let (mut clusters, mut kinds, mut namespaces) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for row in rows {
            match row {
                Row::Command { .. } => {
                    clusters.insert(COMMAND);
                }
                Row::Hit(hit) => {
                    clusters.insert(hit.cluster.as_str());
                    kinds.insert(hit.kind.info().name);
                    namespaces.insert(&*hit.key.namespace);
                }
            }
        }
        let small = FontId::proportional(SMALL);
        let widest = |texts: BTreeSet<&str>, most: f32| {
            let width = |text: &str| {
                ui.fonts_mut(|fonts| {
                    fonts.layout_no_wrap(text.to_owned(), small.clone(), Color32::PLACEHOLDER).size().x
                })
            };
            let widest = texts.into_iter().map(width).fold(0.0, f32::max).min(most);
            // A column with no text takes no room.
            if widest > 0.0 { widest + GAP } else { 0.0 }
        };
        let kind = CLUSTER + widest(clusters, CLUSTER_MOST);
        let namespace = kind + widest(kinds, f32::INFINITY);
        Self { kind, namespace, name: namespace + widest(namespaces, NAMESPACE_MOST) }
    }
}

/// One row. A screen reader reads the words of its columns, and a row with a cut text shows
/// them at the pointer.
fn draw(
    ui: &mut Ui,
    (palette, scene): (&Palette, &Scene<'_>),
    at: Places,
    row: &Row,
    is_cursor: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if is_cursor {
        ui.painter().rect_filled(rect, 4.0, palette.accent_bg);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, palette.bg_raised);
    }
    let (body, small) = (TextStyle::Body.resolve(ui.style()), FontId::proportional(SMALL));
    let end = rect.width() - RIGHT;
    let (label, cut) = match row {
        Row::Command { label, key, .. } => {
            let cuts = [
                text(ui, rect, (CLUSTER, at.kind - GAP), COMMAND, &small, palette.text_faint),
                text(ui, rect, (at.kind, end), label, &body, palette.text),
            ];
            if let Some(key) = key {
                let right = rect.right_center() - egui::vec2(RIGHT, 0.0);
                ui.painter().text(right, Align2::RIGHT_CENTER, key, small, palette.text_muted);
            }
            (format!("Command {label}"), cuts.contains(&true))
        }
        Row::Hit(hit) => {
            let center = rect.left_center() + egui::vec2(11.0, 0.0);
            ui.painter().circle_filled(center, 3.5, scene.colors.color(&hit.cluster, palette));
            let old = hit.stale.map(|stale| old_text(stale.last_contact, scene.clock));
            // The mark of an old store at the right end; the name ends left of it.
            let mark = old.as_ref().map_or(0.0, |old| {
                let right = rect.right_center() - egui::vec2(RIGHT, 0.0);
                let width = ui.painter().text(right, Align2::RIGHT_CENTER, old, small.clone(), palette.warn).width();
                ui.painter().circle_stroke(right - egui::vec2(width + 8.0, 0.0), 3.25, (1.5, palette.warn));
                width + RING + GAP
            });
            let cuts = [
                text(ui, rect, (CLUSTER, at.kind - GAP), hit.cluster.as_str(), &small, palette.text_muted),
                text(ui, rect, (at.kind, at.namespace - GAP), hit.kind.info().name, &small, palette.text_muted),
                text(ui, rect, (at.namespace, at.name - GAP), &hit.key.namespace, &small, palette.text_muted),
                text(ui, rect, (at.name, end - mark), &hit.key.name, &body, palette.text),
            ];
            (hit_label(hit, old.as_deref()), cuts.contains(&true))
        }
    };
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, is_cursor, &label));
    if cut { response.on_hover_text(label) } else { response }
}

/// Paints a text from the place `from` of the row, cut with "…" before the place `to`.
/// Gives true for a cut text.
fn text(ui: &Ui, row: Rect, (from, to): (f32, f32), text: &str, font: &FontId, color: Color32) -> bool {
    if text.is_empty() {
        return false;
    }
    let galley = widgets::one_line(ui, text, font.clone(), color, to - from);
    let (pos, cut) = (egui::pos2(row.left() + from, row.center().y - galley.size().y / 2.0), galley.elided);
    ui.painter().galley(pos, galley, color);
    cut
}

/// The words of a hit for a screen reader: `Secret kinds/web-secret of ams-prod`, and the
/// mark of an old store.
fn hit_label(hit: &Hit, old: Option<&str>) -> String {
    let (kind, key) = (hit.kind.info().name, &hit.key);
    let place = if key.namespace.is_empty() { key.name.to_string() } else { format!("{}/{}", key.namespace, key.name) };
    let old = old.map_or_else(String::new, |old| format!(", {old}"));
    format!("{kind} {place} of {}{old}", hit.cluster.as_str())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use fresnel_core::cluster::ClusterId;
    use fresnel_core::kinds::Kind;
    use fresnel_core::store::ObjectKey;

    use super::*;

    fn hit(cluster: &str, kind: Kind, namespace: &str) -> Row {
        Row::Hit(Hit { cluster: ClusterId::new(cluster), kind, key: ObjectKey::new(namespace, "web"), stale: None })
    }

    /// The places of the columns for each set of rows, and the width of each text at the size
    /// of the columns, in the fonts of one frame.
    fn places(sets: &[Vec<Row>], texts: &[&str]) -> (Vec<Places>, Vec<f32>) {
        let found = RefCell::new((Vec::new(), Vec::new()));
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let width = |text: &str| {
                let font = FontId::proportional(SMALL);
                ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x)
            };
            let shown = sets.iter().map(|rows| Places::of(ui, rows)).collect();
            *found.borrow_mut() = (shown, texts.iter().map(|text| width(text)).collect());
        });
        harness.run();
        drop(harness);
        found.into_inner()
    }

    #[test]
    fn a_column_is_as_wide_as_its_widest_text_up_to_its_limit_and_a_kind_is_never_cut() {
        let long = "a-context-whose-name-is-much-wider-than-its-column";
        let each_kind: Vec<Row> = Kind::ALL.iter().map(|kind| hit("ams", *kind, "shop")).collect();
        let sets = [
            vec![hit("ams", Kind::Pod, "shop")],
            each_kind,
            vec![hit(long, Kind::Pod, long)],
            vec![hit("ams", Kind::Node, "")],
        ];
        let kinds: Vec<&str> = Kind::ALL.iter().map(|kind| kind.info().name).collect();
        let texts: Vec<&str> = ["ams", "Pod", "shop"].into_iter().chain(kinds.iter().copied()).collect();
        let (shown, widths) = places(&sets, &texts);
        let [short, each_kind, long, no_namespace] = shown.as_slice() else { panic!("four places: {shown:?}") };
        let [ams, pod, shop, kinds @ ..] = widths.as_slice() else { panic!("the widths: {widths:?}") };
        let gaps = |widths: &[f32]| widths.iter().map(|width| width + GAP).sum::<f32>();
        assert_eq!(short.kind, CLUSTER + gaps(&[*ams]));
        assert_eq!((short.namespace, short.name), (short.kind + gaps(&[*pod]), short.namespace + gaps(&[*shop])));
        assert!(short.name < 200.0, "the name of a short row has the room: {short:?}");
        let kind_room = each_kind.namespace - GAP - each_kind.kind;
        assert!(kinds.iter().all(|width| *width <= kind_room), "each kind fits {kind_room}: {kinds:?}");
        assert_eq!(long.kind, CLUSTER + CLUSTER_MOST + GAP, "a long cluster is cut");
        assert_eq!(long.name, long.namespace + NAMESPACE_MOST + GAP, "a long namespace is cut");
        assert_eq!(no_namespace.name, no_namespace.namespace, "no namespace, no room");
    }
}
