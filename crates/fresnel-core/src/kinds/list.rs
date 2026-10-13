//! The one list of all kinds (plan section 11.4).
//!
//! The sequence is the priority of a kind for its first list: the small kinds that each
//! view needs come first, and the Events, which can be most of the objects, come last.

use k8s_openapi::api::admissionregistration::v1 as admission;
use k8s_openapi::api::apps::v1 as apps;
use k8s_openapi::api::autoscaling::v2 as autoscaling;
use k8s_openapi::api::batch::v1 as batch;
use k8s_openapi::api::certificates::v1 as certificates;
use k8s_openapi::api::coordination::v1 as coordination;
use k8s_openapi::api::core::v1 as core;
use k8s_openapi::api::discovery::v1 as discovery;
use k8s_openapi::api::flowcontrol::v1 as flowcontrol;
use k8s_openapi::api::networking::v1 as networking;
use k8s_openapi::api::node::v1 as node;
use k8s_openapi::api::policy::v1 as policy;
use k8s_openapi::api::rbac::v1 as rbac;
use k8s_openapi::api::scheduling::v1 as scheduling;
use k8s_openapi::kube_aggregator::pkg::apis::apiregistration::v1 as apiregistration;

use super::{Full, KindInfo, Tier};

/// Takes the kinds that have a store, each one in the form of its tier.
pub(crate) trait Visitor {
    /// A tier A kind, with its object type.
    fn full<K: Full>(&mut self);

    /// A tier B kind.
    fn metadata(&mut self, kind: Kind);
}

/// One line of the list gives the identity of a kind, its tier and its object type.
macro_rules! kinds {
    ($($id:ident: $tier:ident, $object:ty;)*) => {
        /// The identity of one kind. The sequence is the sequence of the list.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Kind {
            $(#[doc = concat!("The kind `", stringify!($id), "`.")] $id,)*
        }

        impl Kind {
            /// Each kind, in the sequence of the list.
            pub const ALL: &'static [Kind] = &[$(Kind::$id,)*];

            /// The description of the kind.
            pub fn info(self) -> &'static KindInfo {
                match self {
                    $(Kind::$id => {
                        const INFO: KindInfo = KindInfo::of::<$object>(Tier::$tier);
                        &INFO
                    })*
                }
            }
        }

        /// Gives each kind with a store to `visitor`, in the sequence of the list.
        pub(crate) fn each_store(visitor: &mut impl Visitor) {
            $(store!($tier, $id, $object, visitor);)*
        }

        $(full!($tier, $id, $object);)*
    };
}

macro_rules! store {
    (A, $id:ident, $object:ty, $visitor:ident) => {
        $visitor.full::<$object>()
    };
    (B, $id:ident, $object:ty, $visitor:ident) => {
        $visitor.metadata(Kind::$id)
    };
    (C, $id:ident, $object:ty, $visitor:ident) => {};
}

macro_rules! full {
    (A, $id:ident, $object:ty) => {
        impl Full for $object {
            const KIND: Kind = Kind::$id;
        }
    };
    ($tier:ident, $id:ident, $object:ty) => {};
}

kinds! {
    Namespace: A, core::Namespace;
    Node: A, core::Node;
    Pod: A, core::Pod;
    Deployment: A, apps::Deployment;
    StatefulSet: A, apps::StatefulSet;
    DaemonSet: A, apps::DaemonSet;
    ReplicaSet: A, apps::ReplicaSet;
    Job: A, batch::Job;
    CronJob: A, batch::CronJob;
    Service: A, core::Service;
    EndpointSlice: A, discovery::EndpointSlice;

    ReplicationController: B, core::ReplicationController;
    ConfigMap: B, core::ConfigMap;
    Secret: B, core::Secret;
    HorizontalPodAutoscaler: B, autoscaling::HorizontalPodAutoscaler;
    PodDisruptionBudget: B, policy::PodDisruptionBudget;
    ResourceQuota: B, core::ResourceQuota;
    LimitRange: B, core::LimitRange;
    PriorityClass: B, scheduling::PriorityClass;
    RuntimeClass: B, node::RuntimeClass;
    MutatingWebhookConfiguration: B, admission::MutatingWebhookConfiguration;
    ValidatingWebhookConfiguration: B, admission::ValidatingWebhookConfiguration;
    Ingress: B, networking::Ingress;
    IngressClass: B, networking::IngressClass;
    NetworkPolicy: B, networking::NetworkPolicy;
    ServiceAccount: B, core::ServiceAccount;
    Role: B, rbac::Role;
    ClusterRole: B, rbac::ClusterRole;
    RoleBinding: B, rbac::RoleBinding;
    ClusterRoleBinding: B, rbac::ClusterRoleBinding;
    ControllerRevision: B, apps::ControllerRevision;
    PodTemplate: B, core::PodTemplate;
    CertificateSigningRequest: B, certificates::CertificateSigningRequest;
    ValidatingAdmissionPolicy: B, admission::ValidatingAdmissionPolicy;
    ValidatingAdmissionPolicyBinding: B, admission::ValidatingAdmissionPolicyBinding;
    ApiService: B, apiregistration::APIService;
    FlowSchema: B, flowcontrol::FlowSchema;
    PriorityLevelConfiguration: B, flowcontrol::PriorityLevelConfiguration;

    Event: A, core::Event;

    Lease: C, coordination::Lease;
    Endpoints: C, core::Endpoints;
}
