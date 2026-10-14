//! The sections of the kinds that have some of their own (plan section 11.4). The file of
//! such a kind in `views` holds them, and this module calls them for its kind. In M2, the
//! Pod has its status text and its node; each other kind has the generic view.

use std::any::Any;
use std::borrow::Cow;

use egui::Ui;
use fresnel_core::kinds::{Describe, ObjectHealth};
use k8s_openapi::api::core::v1::Pod;

use crate::theme::Palette;
use crate::views::pods;

/// The state of an object as the header of the panel shows it.
pub(super) fn state<K: Describe>(object: &K) -> (Cow<'static, str>, ObjectHealth) {
    if let Some(pod) = (object as &dyn Any).downcast_ref::<Pod>() {
        let (text, health) = pods::detail::state(pod);
        return (Cow::Owned(text), health);
    }
    let health = object.health();
    let text = match health {
        ObjectHealth::Healthy => "Healthy",
        ObjectHealth::Progressing => "Progressing",
        ObjectHealth::Warning => "Warning",
        ObjectHealth::Failed => "Failed",
    };
    (Cow::Borrowed(text), health)
}

/// The lines of the metadata section that a kind adds.
pub(super) fn fields<K: Describe>(ui: &mut Ui, palette: &Palette, object: &K) {
    if let Some(pod) = (object as &dyn Any).downcast_ref::<Pod>() {
        pods::detail::fields(ui, palette, pod);
    }
}
