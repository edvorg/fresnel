//! The volumes of a spec: the name, the type of the source and its objects as links.

use egui::{RichText, Ui};

use super::texts::{self, Part};
use super::{Place, section};
use crate::format::short;
use crate::theme::SMALL;
use crate::views::detail::fields::{ITEM_LIMIT, VALUE_LIMIT, field, more};
use crate::views::objects::Link;
use k8s_openapi::api::core::v1::PodSpec;

/// Draws the section. A spec with no volume has none. Gives the object of a link that the
/// user clicked.
pub(super) fn show(ui: &mut Ui, place: &Place<'_>, spec: &PodSpec) -> Option<Link> {
    let volumes = spec.volumes.as_deref().unwrap_or_default();
    if volumes.is_empty() {
        return None;
    }
    let palette = place.palette;
    section(ui, palette, "VOLUMES");
    let mut clicked = None;
    for volume in volumes.iter().take(ITEM_LIMIT) {
        let (kind, parts) = texts::volume(volume);
        field(ui, palette, &volume.name, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new(kind).size(SMALL).color(palette.text_faint));
                for part in parts {
                    match part {
                        Part::Object(kind, name) => {
                            let text = format!("{} {name}", kind.info().name);
                            if let Some(link) = place.link(ui, &text, kind, name) {
                                clicked = Some(link);
                            }
                        }
                        Part::Text(text) => {
                            ui.label(short(&text, VALUE_LIMIT));
                        }
                    }
                }
            });
        });
    }
    more(ui, palette, volumes.len());
    ui.add_space(10.0);
    clicked
}
