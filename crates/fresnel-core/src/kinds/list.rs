//! The one list of all kinds (plan section 11.4).
//!
//! The sequence is the priority of a kind for its first list: the small kinds that each
//! view needs come first, and the Events, which can be most of the objects, come last.
//! It is also the sequence of the kinds in each group of the sidebar.

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

use super::{Describe, KindInfo};

/// Takes the object type of one kind: [`Kind::visit`] turns a kind into its type.
pub trait Visit {
    /// What the visit gives.
    type Output;

    /// Takes the type of the kind.
    fn visit<K: Describe>(self) -> Self::Output;
}

/// One line of the list gives the identity of a kind and its object type. The file of the
/// kind describes the type.
macro_rules! kinds {
    ($($id:ident: $object:ty;)*) => {
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
                        const INFO: KindInfo = KindInfo::of::<$object>();
                        &INFO
                    })*
                }
            }

            /// Gives the object type of the kind to `visitor`.
            pub fn visit<V: Visit>(self, visitor: V) -> V::Output {
                match self {
                    $(Kind::$id => visitor.visit::<$object>(),)*
                }
            }
        }

        // The identity in the description of a type is the identity of its line.
        $(const _: () = assert!(
            matches!(<$object as Describe>::ID, Kind::$id),
            concat!("the description of `", stringify!($object), "` names an other kind"),
        );)*
    };
}

kinds! {
    Node: core::Node;
    Namespace: core::Namespace;
    Pod: core::Pod;
    Deployment: apps::Deployment;
    StatefulSet: apps::StatefulSet;
    DaemonSet: apps::DaemonSet;
    ReplicaSet: apps::ReplicaSet;
    ReplicationController: core::ReplicationController;
    Job: batch::Job;
    CronJob: batch::CronJob;
    Service: core::Service;
    EndpointSlice: discovery::EndpointSlice;
    Endpoints: core::Endpoints;

    ConfigMap: core::ConfigMap;
    Secret: core::Secret;
    HorizontalPodAutoscaler: autoscaling::HorizontalPodAutoscaler;
    PodDisruptionBudget: policy::PodDisruptionBudget;
    ResourceQuota: core::ResourceQuota;
    LimitRange: core::LimitRange;
    PriorityClass: scheduling::PriorityClass;
    RuntimeClass: node::RuntimeClass;
    Lease: coordination::Lease;
    MutatingWebhookConfiguration: admission::MutatingWebhookConfiguration;
    ValidatingWebhookConfiguration: admission::ValidatingWebhookConfiguration;
    Ingress: networking::Ingress;
    IngressClass: networking::IngressClass;
    NetworkPolicy: networking::NetworkPolicy;
    ServiceAccount: core::ServiceAccount;
    Role: rbac::Role;
    ClusterRole: rbac::ClusterRole;
    RoleBinding: rbac::RoleBinding;
    ClusterRoleBinding: rbac::ClusterRoleBinding;
    ControllerRevision: apps::ControllerRevision;
    PodTemplate: core::PodTemplate;
    CertificateSigningRequest: certificates::CertificateSigningRequest;
    ValidatingAdmissionPolicy: admission::ValidatingAdmissionPolicy;
    ValidatingAdmissionPolicyBinding: admission::ValidatingAdmissionPolicyBinding;
    ApiService: apiregistration::APIService;
    FlowSchema: flowcontrol::FlowSchema;
    PriorityLevelConfiguration: flowcontrol::PriorityLevelConfiguration;

    Event: core::Event;
}
