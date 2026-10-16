//! The parts of a pod spec that a pod and the template of a workload share (plan section
//! 11.2, `research/01` section 3.3): the containers with their image, state, ports, probes,
//! resources, environment and mounts, and the volumes. A ConfigMap, a Secret or a
//! ServiceAccount that the spec names is a link to its panel.
//!
//! A pod has the status of each container; a template has none, so its blocks have no state.
//!
//! - `containers`: one block for each container. `volumes`: the volumes.
//! - `scheduling`: the ServiceAccount, the node selector, the tolerations and the classes.
//! - `texts`: the texts of a probe, a port, a mount, a state and a source, as kubectl prints them.

mod containers;
pub(crate) mod scheduling;
pub(crate) mod texts;
mod volumes;

pub(crate) use containers::forward_button;

use egui::Ui;
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::{PodSpec, PodStatus};

use crate::format::{Clock, short};
use crate::theme::Palette;
use crate::views::detail::fields::{VALUE_LIMIT, heading};
use crate::views::detail::sections::Pick;
use crate::views::objects::Link;

/// Where the objects of a spec are, and what the parts need to draw them.
pub(crate) struct Place<'a> {
    pub(crate) palette: &'a Palette,
    pub(crate) clock: &'a Clock,
    /// The cluster of the pod or of the workload.
    pub(crate) cluster: &'a ClusterId,
    /// Its namespace: a spec names objects of its own namespace only.
    pub(crate) namespace: &'a str,
    /// True when the object forwards a port (plan section 12.3): each TCP port of a container
    /// then has a Forward button. A CronJob has none: its Jobs come and go.
    pub(crate) forwards: bool,
}

impl Place<'_> {
    /// A link to an object of the namespace. Gives the object when the user clicked it.
    pub(crate) fn link(&self, ui: &mut Ui, text: &str, kind: Kind, name: &str) -> Option<Link> {
        let clicked = ui.link(short(text, VALUE_LIMIT)).clicked();
        clicked.then(|| Link { cluster: self.cluster.clone(), kind, key: ObjectKey::new(self.namespace, name) })
    }
}

/// The containers and the volumes of a spec. `status` is the status of a pod, `None` for
/// a template. Gives the link or the logs of a container that the user clicked.
pub(crate) fn show(ui: &mut Ui, place: &Place<'_>, spec: &PodSpec, status: Option<&PodStatus>) -> Option<Pick> {
    let picked = containers::show(ui, place, spec, status);
    volumes::show(ui, place, spec).map(Pick::Open).or(picked)
}

/// The heading of a section of the spec, with a space after the section before it.
fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(4.0);
    heading(ui, palette, title);
}
