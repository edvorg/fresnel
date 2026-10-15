//! The confirm rule `type-name` of a cluster (plan section 12.5): the confirm button of a
//! change waits until the user typed the name of each such cluster of its targets. A
//! cluster of production thus gets one more step than a click on a default button.

use std::collections::BTreeSet;

use egui::{RichText, TextEdit, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::cluster::ClusterId;
use fresnel_core::config::ConfirmRule;

use crate::theme::Palette;
use crate::widgets;

/// The text that the user typed.
#[derive(Debug, Default)]
pub(crate) struct TypedName {
    text: String,
}

impl TypedName {
    /// Draws the field when a cluster of `clusters` has the rule. Gives the reason that
    /// keeps the confirm button off, if the text does not name each such cluster.
    pub(crate) fn show<'a>(
        &mut self,
        ui: &mut Ui,
        palette: &Palette,
        view: &CoreView,
        clusters: impl Iterator<Item = &'a ClusterId>,
    ) -> Option<String> {
        let wanted: BTreeSet<&str> = clusters
            .filter(|cluster| view.config.confirm_rule(cluster) == ConfirmRule::TypeName)
            .map(ClusterId::as_str)
            .collect();
        if wanted.is_empty() {
            return None;
        }
        let names = wanted.iter().copied().collect::<Vec<_>>().join(" ");
        ui.add_space(4.0);
        let prompt = format!("Type {names} to confirm a change in it:");
        ui.label(RichText::new(prompt).color(palette.warn));
        let field = ui.add(TextEdit::singleline(&mut self.text).desired_width(360.0));
        widgets::name(&field, "Name of the cluster");
        let typed: BTreeSet<&str> = self.text.split([' ', ',']).filter(|word| !word.is_empty()).collect();
        (!wanted.is_subset(&typed)).then(|| format!("The settings ask for the name of {names} before a change."))
    }
}
