//! The sidebar: the pages of the center of the window. The kinds come in the groups of
//! plan section 11.3, each with the count of its objects in the selected clusters.
//!
//! A count comes from the facts of the stores that the core gives each frame, so the
//! sidebar holds no objects. A kind with no store that is always on (tier C) has a count
//! only while its page holds the store of its objects.

use egui::{Align2, Label, RichText, ScrollArea, Sense, TextStyle, Ui, WidgetInfo, WidgetType};
use fresnel_core::app::CoreView;
use fresnel_core::kinds::{Kind, Sidebar, Tier};

use crate::format;
use crate::theme::{self, Palette, ROW_HEIGHT, SMALL};

pub(super) const WIDTH: f32 = 196.0;

/// What the center of the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    /// The table of one kind.
    Kind(Kind),
    Connections,
}

/// Draws the entries. A click on an entry opens its page.
pub(super) fn show(ui: &mut Ui, view: &CoreView, open: &mut Page) {
    let palette = theme::palette(ui);
    ui.add_space(8.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    if entry(ui, palette, "Connections", None, *open == Page::Connections).clicked() {
        *open = Page::Connections;
    }
    ScrollArea::vertical().id_salt("sidebar").auto_shrink(false).show(ui, |ui| {
        for group in Sidebar::ALL {
            ui.add_space(10.0);
            let title = RichText::new(group.title().to_uppercase()).size(SMALL).color(palette.text_faint);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.add(Label::new(title).selectable(false));
            });
            ui.add_space(2.0);
            for kind in group.kinds() {
                let count = count(view, kind).map(format::thousands);
                if entry(ui, palette, kind.info().title, count.as_deref(), *open == Page::Kind(kind)).clicked() {
                    *open = Page::Kind(kind);
                }
            }
        }
        ui.add_space(8.0);
    });
}

/// The count of the objects of a kind in the selected clusters. `None` while no selected
/// cluster has a store of the kind.
fn count(view: &CoreView, kind: Kind) -> Option<u64> {
    let selected = view.clusters.iter().filter(|row| row.selected).filter_map(|row| row.data.as_ref());
    let counts = selected.filter_map(|data| match kind.info().tier {
        Tier::A | Tier::B => data.store(kind).map(|store| store.items),
        Tier::C => data.objects.count(kind),
    });
    counts.map(|count| count as u64).reduce(|sum, count| sum + count)
}

/// One entry. The entries are one choice of many, so a screen reader gets a radio button.
/// A title that is too long for the sidebar is cut, and shows in full at the pointer.
fn entry(ui: &mut Ui, palette: &Palette, title: &str, count: Option<&str>, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, title));
    if let Some(count) = count {
        // A screen reader reads the count as the value of the entry.
        response.ctx.accesskit_node_builder(response.id, |node| node.set_value(count));
    }
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 4.0, palette.accent_bg);
    } else if response.hovered() {
        painter.rect_filled(rect, 4.0, palette.bg_raised);
    }
    let right = rect.right_center() - egui::vec2(8.0, 0.0);
    let count_width = count.map_or(0.0, |count| {
        let font = TextStyle::Small.resolve(ui.style());
        painter.text(right, Align2::RIGHT_CENTER, count, font, palette.text_faint).width() + 6.0
    });
    let color = if selected { palette.text } else { palette.text_muted };
    let font = TextStyle::Body.resolve(ui.style());
    let room = (rect.width() - 18.0 - count_width - 8.0).max(0.0);
    let galley = painter.layout(title.to_owned(), font, color, f32::INFINITY);
    let cut = galley.size().x > room;
    let left = rect.left_center() + egui::vec2(10.0, -galley.size().y / 2.0);
    let clip = egui::Rect::from_min_size(rect.min, egui::vec2(10.0 + room, rect.height()));
    ui.painter().with_clip_rect(clip.intersect(ui.clip_rect())).galley(left, galley, color);
    if cut { response.on_hover_text(title) } else { response }
}
