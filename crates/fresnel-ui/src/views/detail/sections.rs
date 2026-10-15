//! The sections of the kinds that have some of their own (plan section 11.4). The file of
//! such a kind in `views` holds them, and this module calls them for its kind; each other
//! kind has the generic view.
//!
//! A kind has two places: its details after the metadata, and its parts after the related
//! objects, such as the containers, the volumes, the revisions or the Jobs of a CronJob.
//! A section that reads the full objects of an other kind names that kind in `also_shown`.

use std::any::Any;
use std::borrow::Cow;

use egui::Ui;
use fresnel_core::app::{ClusterData, CoreView};
use fresnel_core::kinds::{Describe, Kind, ObjectHealth};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Pod, ReplicationController};

use super::fields::VALUE_LIMIT;
use crate::format::{Clock, short};
use crate::theme::Palette;
use crate::views::act::Ask;
use crate::views::objects::Link;
use crate::views::pod_spec::Place;
use crate::views::{access, config, network, pods, workloads};
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

    /// A link with this text to an object of the cluster of the panel. A kind with no
    /// namespace takes none. Gives the object when the user clicked it.
    pub(crate) fn link(&self, ui: &mut Ui, text: &str, kind: Kind, namespace: &str, name: &str) -> Option<Link> {
        let namespace = if kind.info().namespaced { namespace } else { "" };
        let clicked = ui.link(short(text, VALUE_LIMIT)).clicked();
        clicked.then(|| Link { cluster: self.id.cluster.clone(), kind, key: ObjectKey::new(namespace, name) })
    }

    /// An object that an other object names with the name of its kind, as "Deployment web":
    /// a link when the kind is in the scope of the app, else a text.
    pub(crate) fn reference(&self, ui: &mut Ui, kind: &str, namespace: &str, name: &str) -> Option<Link> {
        let text = format!("{kind} {name}");
        match Kind::named(kind) {
            Some(kind) => self.link(ui, &text, kind, namespace, name),
            None => {
                ui.label(short(&text, VALUE_LIMIT));
                None
            }
        }
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
        Kind::Service => of(object).and_then(|service| network::service::details(ui, cx, service)),
        Kind::EndpointSlice => of(object).and_then(|slice| network::endpoints::slice_details(ui, cx, slice)),
        Kind::Endpoints => of(object).and_then(|endpoints| network::endpoints::details(ui, cx, endpoints)),
        Kind::Ingress => of(object).and_then(|ingress| network::ingress::details(ui, cx, ingress)),
        Kind::IngressClass => of(object).and_then(|class| network::ingress::class_details(ui, cx, class)),
        Kind::NetworkPolicy => of(object).and_then(|policy| network::policy::details(ui, cx, policy)),
        Kind::HorizontalPodAutoscaler => of(object).and_then(|scaler| config::autoscaler::details(ui, cx, scaler)),
        Kind::PodDisruptionBudget => of(object).and_then(|budget| config::disruption::details(ui, cx, budget)),
        Kind::ResourceQuota => of(object).and_then(|quota| config::quota::quota_details(ui, cx, quota)),
        Kind::LimitRange => of(object).and_then(|range| config::quota::range_details(ui, cx, range)),
        Kind::PriorityClass => of(object).and_then(|class| config::classes::priority_details(ui, cx, class)),
        Kind::RuntimeClass => of(object).and_then(|class| config::classes::runtime_details(ui, cx, class)),
        Kind::Lease => of(object).and_then(|lease| config::lease::details(ui, cx, lease)),
        Kind::MutatingWebhookConfiguration => of(object).and_then(|hooks| config::webhooks::mutating(ui, cx, hooks)),
        Kind::ValidatingWebhookConfiguration => {
            of(object).and_then(|hooks| config::webhooks::validating(ui, cx, hooks))
        }
        Kind::ServiceAccount => of(object).and_then(|account| access::account::details(ui, cx, account)),
        Kind::Role => of(object).and_then(|role| access::rules::role(ui, cx, role)),
        Kind::ClusterRole => of(object).and_then(|role| access::rules::cluster_role(ui, cx, role)),
        Kind::RoleBinding => of(object).and_then(|binding| access::bindings::role_binding(ui, cx, binding)),
        Kind::ClusterRoleBinding => {
            of(object).and_then(|binding| access::bindings::cluster_role_binding(ui, cx, binding))
        }
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
        Kind::Service => network::service::parts(ui, cx).map(Pick::Open),
        Kind::ServiceAccount => access::account::parts(ui, cx).map(Pick::Open),
        _ => None,
    }
}

/// The kinds whose full objects the sections of a kind read besides the objects of the kind
/// itself: the page of the kind names them to the core (`Command::ShowKinds`).
pub(crate) fn also_shown(kind: Kind) -> &'static [Kind] {
    match kind {
        // The last runs of a CronJob show the state of each of its Jobs.
        Kind::CronJob => &[Kind::Job],
        Kind::ServiceAccount => &[Kind::RoleBinding, Kind::ClusterRoleBinding],
        _ => &[],
    }
}

/// The object as its own type, when it is of the type `T`.
fn of<T: 'static, K: Describe>(object: &K) -> Option<&T> {
    (object as &dyn Any).downcast_ref()
}
