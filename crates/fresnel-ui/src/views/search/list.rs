//! The list of the quick search: the commands of the command table that match the text,
//! then the hits. Each hit shows its cluster, kind, namespace and name, and the mark of a
//! store that is not live.

use egui::{Align2, Color32, FontId, Rect, ScrollArea, Sense, TextStyle, Ui, WidgetInfo, WidgetType};
use fresnel_core::commands::{self, CommandId};
use fresnel_core::search::{Found, Hit};

use super::Choice;
use crate::shell::Scene;
use crate::theme::{self, Palette, ROW_HEIGHT, SMALL};
use crate::views::detail::actions;
use crate::views::objects::{Link, old_text};

/// The height of the list. More rows scroll.
const HEIGHT: f32 = 360.0;
/// The places of the columns of a row: cluster, kind, namespace and name. The name takes
/// the rest of the row, and the mark of an old store its right end.
const CLUSTER: f32 = 22.0;
const KIND: f32 = 170.0;
const NAMESPACE: f32 = 320.0;
const NAME: f32 = 470.0;
const MARK: f32 = 150.0;

/// One row of the list.
pub(super) enum Row {
    Command { id: CommandId, label: &'static str, key: Option<String> },
    Hit(Hit),
}

impl Row {
    pub(super) fn choice(&self) -> Choice {
        match self {
            Self::Command { id, .. } => Choice::Run(*id),
            Self::Hit(hit) => Choice::Open(Link { cluster: hit.cluster.clone(), kind: hit.kind, key: hit.key.clone() }),
        }
    }
}

/// The rows for a text: each command that this version runs and whose label has each word
/// of the text, then the hits of the result. `key` gives the text of the key of a command.
pub(super) fn rows(found: Option<&Found>, text: &str, key: impl Fn(CommandId) -> Option<String>) -> Vec<Row> {
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    let runs = |id: CommandId| match id {
        CommandId::QuickSearch => false,
        CommandId::Back | CommandId::Forward | CommandId::Filter => true,
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
    let mut clicked = None;
    let list = ScrollArea::vertical().id_salt("quick search list").max_height(HEIGHT).auto_shrink([false, true]);
    list.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
        for index in range {
            let Some(row) = rows.get(index) else {
                continue;
            };
            let response = draw(ui, palette, scene, row, index == cursor);
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

/// One row. A screen reader reads the words of its columns.
fn draw(ui: &mut Ui, palette: &Palette, scene: &Scene<'_>, row: &Row, is_cursor: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if is_cursor {
        ui.painter().rect_filled(rect, 4.0, palette.accent_bg);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, palette.bg_raised);
    }
    let (body, small) = (TextStyle::Body.resolve(ui.style()), FontId::proportional(SMALL));
    let label = match row {
        Row::Command { label, key, .. } => {
            text(ui, rect, (CLUSTER, KIND), "Command", &small, palette.text_faint);
            text(ui, rect, (KIND, rect.width()), label, &body, palette.text);
            if let Some(key) = key {
                let right = rect.right_center() - egui::vec2(8.0, 0.0);
                ui.painter().text(right, Align2::RIGHT_CENTER, key, small, palette.text_muted);
            }
            format!("Command {label}")
        }
        Row::Hit(hit) => {
            let center = rect.left_center() + egui::vec2(11.0, 0.0);
            ui.painter().circle_filled(center, 3.5, scene.colors.color(&hit.cluster, palette));
            text(ui, rect, (CLUSTER, KIND), hit.cluster.as_str(), &small, palette.text_muted);
            text(ui, rect, (KIND, NAMESPACE), hit.kind.info().name, &small, palette.text_muted);
            text(ui, rect, (NAMESPACE, NAME), &hit.key.namespace, &small, palette.text_muted);
            let old = hit.stale.map(|stale| old_text(stale.last_contact, scene.clock));
            let name_end = rect.width() - if old.is_some() { MARK } else { 0.0 };
            text(ui, rect, (NAME, name_end), &hit.key.name, &body, palette.text);
            if let Some(old) = &old {
                let right = rect.right_center() - egui::vec2(8.0, 0.0);
                let width = ui.painter().text(right, Align2::RIGHT_CENTER, old, small, palette.warn).width();
                ui.painter().circle_stroke(right - egui::vec2(width + 8.0, 0.0), 3.25, (1.5, palette.warn));
            }
            hit_label(hit, old.as_deref())
        }
    };
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, is_cursor, &label));
    response
}

/// Paints a text from the place `from` of the row, cut at the place `to`.
fn text(ui: &Ui, row: Rect, (from, to): (f32, f32), text: &str, font: &FontId, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), color);
    let left = row.left() + from;
    let clip = Rect::from_x_y_ranges(left..=(row.left() + to - 8.0).max(left), row.y_range());
    let pos = egui::pos2(left, row.center().y - galley.size().y / 2.0);
    ui.painter().with_clip_rect(clip.intersect(ui.clip_rect())).galley(pos, galley, color);
}

/// The words of a hit for a screen reader: `Secret kinds/web-secret of ams-prod`, and the
/// mark of an old store.
fn hit_label(hit: &Hit, old: Option<&str>) -> String {
    let (kind, key) = (hit.kind.info().name, &hit.key);
    let place = if key.namespace.is_empty() { key.name.to_string() } else { format!("{}/{}", key.namespace, key.name) };
    let old = old.map_or_else(String::new, |old| format!(", {old}"));
    format!("{kind} {place} of {}{old}", hit.cluster.as_str())
}
