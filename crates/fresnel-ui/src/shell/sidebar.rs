//! The sidebar: the pages of the center of the window. The kinds come in the groups of
//! plan section 11.3, each with the count of its objects in the selected clusters, after
//! the overview of the group when it has one.
//!
//! A count comes from the facts of the stores that the core gives each frame, so the
//! sidebar holds no objects. A kind with no store that is always on (tier C) has a count
//! only while its page holds the store of its objects.
//!
//! A count is current when each selected cluster has a live store of the kind with a
//! complete list. Any other count has the freshness mark of the rows: a store that is
//! forbidden has no object, and its "0" must not look like a kind with none (design rules
//! 4 and 5). The page of the kind gives the reason.

use egui::{Align2, Label, RichText, ScrollArea, Sense, TextStyle, Ui, WidgetInfo, WidgetType};
use fresnel_core::app::CoreView;
use fresnel_core::kinds::{Kind, Sidebar};

use crate::format;
use crate::theme::{self, Palette, ROW_HEIGHT, SMALL};
use crate::views::overview::Overview;

pub(super) const WIDTH: f32 = 196.0;

/// What the center of the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    /// The table of one kind.
    Kind(Kind),
    Overview(Overview),
    Connections,
    Settings,
}

/// Draws the entries. A click on an entry opens its page, and so do Space and Enter on the
/// entry of the focus. `focus` gives the focus of the keys to the entry of the open page.
pub(super) fn show(ui: &mut Ui, view: &CoreView, open: &mut Page, focus: bool) {
    let palette = theme::palette(ui);
    ui.add_space(8.0);
    ui.spacing_mut().item_spacing.y = 1.0;
    let take = |response: egui::Response, page: Page, open: &mut Page| {
        if focus && *open == page {
            response.request_focus();
            response.scroll_to_me(None);
        }
        if response.clicked() {
            *open = page;
        }
    };
    for (page, title) in [(Page::Connections, "Connections"), (Page::Settings, "Settings")] {
        take(entry(ui, palette, (title, title), None, *open == page), page, open);
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
            if let Some(overview) = Overview::of(group) {
                let page = Page::Overview(overview);
                take(entry(ui, palette, ("Overview", overview.title()), None, *open == page), page, open);
            }
            for kind in group.kinds() {
                let (title, page) = (kind.info().title, Page::Kind(kind));
                take(entry(ui, palette, (title, title), count(view, kind), *open == page), page, open);
            }
        }
        ui.add_space(8.0);
    });
}

/// The count of the objects of a kind in the selected clusters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Count {
    objects: u64,
    /// True when each selected cluster has a live store of the kind with a complete list.
    current: bool,
}

impl Count {
    /// The text of the count, and the value that a screen reader reads.
    fn texts(self) -> (String, String) {
        let text = format::thousands(self.objects);
        let value = if self.current { text.clone() } else { format!("{text}, not current") };
        (text, value)
    }
}

/// The count of a kind. `None` while no selected cluster has a store of the kind.
fn count(view: &CoreView, kind: Kind) -> Option<Count> {
    let selected = view.clusters.iter().filter(|row| row.selected);
    let stores: Vec<_> = selected.map(|row| row.data.as_ref().and_then(|data| data.store(kind))).collect();
    let current = stores.iter().all(|store| store.is_some_and(|store| store.listed && store.state.is_live()));
    let objects = stores.iter().flatten().map(|store| store.items as u64).reduce(|sum, items| sum + items)?;
    Some(Count { objects, current })
}

/// One entry with its title and its name for a screen reader: the overview of a group has
/// the title "Overview" and the name of its group. The entries are one choice of many, so a
/// screen reader gets a radio button. A title that is too long for the sidebar is cut, and
/// shows in full at the pointer.
fn entry(
    ui: &mut Ui,
    palette: &Palette,
    (title, name): (&str, &str),
    count: Option<Count>,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, selected, name));
    let texts = count.map(Count::texts);
    if let Some((_, value)) = &texts {
        // A screen reader reads the count as the value of the entry.
        response.ctx.accesskit_node_builder(response.id, |node| node.set_value(value.as_str()));
    }
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 4.0, palette.accent_bg);
    } else if response.hovered() {
        painter.rect_filled(rect, 4.0, palette.bg_raised);
    }
    if response.has_focus() {
        painter.rect_stroke(rect, 4.0, (1.5, palette.accent), egui::StrokeKind::Inside);
    }
    let right = rect.right_center() - egui::vec2(8.0, 0.0);
    let count_width = texts.as_ref().map_or(0.0, |(text, _)| {
        let font = TextStyle::Small.resolve(ui.style());
        let width = painter.text(right, Align2::RIGHT_CENTER, text, font, palette.text_faint).width() + 6.0;
        if count.is_some_and(|count| !count.current) {
            // The freshness mark of the rows, left of the count.
            let center = right - egui::vec2(width + 2.0, 0.0);
            painter.circle_stroke(center, 3.25, (1.5, palette.warn));
            return width + 12.0;
        }
        width
    });
    let color = if selected { palette.text } else { palette.text_muted };
    let font = TextStyle::Body.resolve(ui.style());
    let room = (rect.width() - 18.0 - count_width - 8.0).max(0.0);
    let galley = crate::widgets::one_line(ui, title, font, color, room);
    let cut = galley.elided;
    let left = rect.left_center() + egui::vec2(10.0, -galley.size().y / 2.0);
    ui.painter().galley(left, galley, color);
    match count.filter(|count| !count.current) {
        Some(_) => {
            response.on_hover_text(format!("{title}: the count can be old or not complete. The page tells why."))
        }
        None if cut => response.on_hover_text(title),
        None => response,
    }
}
