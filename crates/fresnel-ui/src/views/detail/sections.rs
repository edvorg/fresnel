//! The sections of the kinds that have some of their own (plan section 11.4). The file of
//! such a kind in `views` holds them, and this module calls them for its kind; each other
//! kind has the generic view.
//!
//! A kind has two places: its details after the metadata, and its parts after the related
//! objects, such as the containers, the volumes, the revisions or the Jobs of a CronJob.

use std::any::Any;
use std::borrow::Cow;

use egui::Ui;
use fresnel_core::app::{ClusterData, CoreView};
use fresnel_core::kinds::{Describe, Kind, ObjectHealth};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Pod, ReplicationController};

use crate::format::Clock;
use crate::theme::Palette;
use crate::views::act::Ask;
use crate::views::objects::Link;
use crate::views::pod_spec::Place;
use crate::views::{pods, workloads};
use crate::widgets::table::model::ObjectId;

/// What a section of a kind reads besides its object.
pub(crate) struct Context<'a> {
    pub(crate) palette: &'a Palette,
    pub(crate) view: &'a CoreView,
    pub(crate) clock: &'a Clock,
    /// The identity of the object of the panel.
    pub(crate) id: &'a ObjectId,
    /// The data of its cluster. `None` for a cluster that is not connected.
    pub(crate) data: Option<&'a ClusterData>,
}

impl Context<'_> {
    /// The place of the objects that a pod spec of the object names.
    pub(crate) fn place(&self) -> Place<'_> {
        Place { palette: self.palette, clock: self.clock, cluster: &self.id.cluster, namespace: &self.id.key.namespace }
    }
}

/// What the user did in a section.
pub(crate) enum Pick {
    /// A click on a link.
    Open(Link),
    /// A click on an action, for example "Roll back" of one revision.
    Ask(Ask),
}

impl From<Link> for Pick {
    fn from(link: Link) -> Self {
        Self::Open(link)
    }
}

/// The state of an object as the header of the panel shows it.
pub(super) fn state<K: Describe>(object: &K) -> (Cow<'static, str>, ObjectHealth) {
    if let Some(pod) = of::<Pod, K>(object) {
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

/// The details of the kind, after the metadata.
pub(super) fn details<K: Describe>(ui: &mut Ui, cx: &Context<'_>, object: &K) -> Option<Pick> {
    let link = match K::ID {
        Kind::Pod => of(object).and_then(|pod| pods::detail::details(ui, cx, pod)),
        Kind::Deployment => of(object).and_then(|set| workloads::deployment::details(ui, cx, set)),
        Kind::StatefulSet => of(object).and_then(|set| workloads::stateful_set::details(ui, cx, set)),
        Kind::DaemonSet => of(object).and_then(|set| workloads::daemon_set::details(ui, cx, set)),
        Kind::ReplicaSet => of(object).and_then(|set| workloads::replica_set::details(ui, cx, set)),
        Kind::ReplicationController => {
            of(object).and_then(|set| workloads::replica_set::controller_details(ui, cx, set))
        }
        Kind::Job => of(object).and_then(|job| workloads::job::details(ui, cx, job)),
        Kind::CronJob => of(object).and_then(|job| workloads::cron_job::details(ui, cx, job)),
        _ => None,
    };
    link.map(Pick::Open)
}

/// The parts of the kind, after the related objects.
pub(super) fn parts<K: Describe>(ui: &mut Ui, cx: &Context<'_>, object: &K) -> Option<Pick> {
    match K::ID {
        Kind::Pod => of::<Pod, K>(object).and_then(|pod| pods::detail::parts(ui, cx, pod)).map(Pick::Open),
        Kind::Deployment => of::<Deployment, K>(object).and_then(|set| workloads::deployment::parts(ui, cx, set)),
        Kind::StatefulSet => of::<StatefulSet, K>(object).and_then(|set| workloads::stateful_set::parts(ui, cx, set)),
        Kind::DaemonSet => of::<DaemonSet, K>(object).and_then(|set| workloads::daemon_set::parts(ui, cx, set)),
        Kind::ReplicaSet => of::<ReplicaSet, K>(object).and_then(|set| workloads::replica_set::parts(ui, cx, set)),
        Kind::ReplicationController => {
            of::<ReplicationController, K>(object).and_then(|set| workloads::replica_set::controller_parts(ui, cx, set))
        }
        Kind::Job => of::<Job, K>(object).and_then(|job| workloads::job::parts(ui, cx, job)),
        Kind::CronJob => of::<CronJob, K>(object).and_then(|job| workloads::cron_job::parts(ui, cx, job)),
        _ => None,
    }
}

/// The object as its own type, when it is of the type `T`.
fn of<T: 'static, K: Describe>(object: &K) -> Option<&T> {
    (object as &dyn Any).downcast_ref()
}
