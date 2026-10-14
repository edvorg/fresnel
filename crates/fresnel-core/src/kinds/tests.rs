use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::Pod;

use super::*;

fn names(tier: Tier) -> Vec<&'static str> {
    Kind::ALL.iter().map(|kind| kind.info()).filter(|info| info.tier == tier).map(|info| info.name).collect()
}

#[test]
fn the_tiers_hold_the_kinds_of_the_plan() {
    let mut full = names(Tier::A);
    full.sort_unstable();
    let expected = [
        "CronJob",
        "DaemonSet",
        "Deployment",
        "EndpointSlice",
        "Event",
        "Job",
        "Namespace",
        "Node",
        "Pod",
        "ReplicaSet",
        "Service",
        "StatefulSet",
    ];
    assert_eq!(full, expected, "plan section 9.3");
    assert_eq!(names(Tier::B).len(), 27, "each other kind of plan section 11.3");
    assert_eq!(names(Tier::C), ["Endpoints", "Lease"], "they change all the time");
}

/// Plan section 11.3, in the sequence of the plan.
#[test]
fn the_sidebar_has_each_kind_of_the_plan_in_its_group() {
    let titles = |group: Sidebar| group.kinds().map(|kind| kind.info().title).collect::<Vec<_>>().join(", ");
    let expected = [
        (Sidebar::Cluster, "Nodes, Namespaces, Events"),
        (
            Sidebar::Workloads,
            "Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, ReplicationControllers, Jobs, CronJobs",
        ),
        (
            Sidebar::Config,
            "ConfigMaps, Secrets, HorizontalPodAutoscalers, PodDisruptionBudgets, ResourceQuotas, LimitRanges, \
             PriorityClasses, RuntimeClasses, Leases, MutatingWebhookConfigurations, ValidatingWebhookConfigurations",
        ),
        (Sidebar::Network, "Services, EndpointSlices, Endpoints, Ingresses, IngressClasses, NetworkPolicies"),
        (Sidebar::AccessControl, "ServiceAccounts, Roles, ClusterRoles, RoleBindings, ClusterRoleBindings"),
        (
            Sidebar::Other,
            "ControllerRevisions, PodTemplates, CertificateSigningRequests, ValidatingAdmissionPolicies, \
             ValidatingAdmissionPolicyBindings, APIServices, FlowSchemas, PriorityLevelConfigurations",
        ),
    ];
    for (group, kinds) in expected {
        assert_eq!(titles(group), kinds, "{}", group.title());
    }
    assert_eq!(Sidebar::ALL.iter().map(|group| group.kinds().count()).sum::<usize>(), Kind::ALL.len());
}

#[test]
fn each_kind_has_its_own_path_and_no_store_holds_the_data_of_a_secret() {
    let paths: BTreeSet<String> = Kind::ALL.iter().map(|kind| kind.info().path()).collect();
    assert_eq!(paths.len(), Kind::ALL.len());
    // An Event names the kind of its object, so a name is one kind.
    assert!(Kind::ALL.iter().all(|kind| Kind::named(kind.info().name) == Some(*kind)));
    assert_eq!(Kind::named("Widget"), None, "a kind out of scope");
    assert_eq!(Kind::Pod.info().path(), "/api/v1/pods");
    assert_eq!(Kind::Deployment.info().path(), "/apis/apps/v1/deployments");
    assert_eq!(Kind::ApiService.info().path(), "/apis/apiregistration.k8s.io/v1/apiservices");
    assert_eq!(Kind::Event.info().path_in("shop"), "/api/v1/namespaces/shop/events");
    assert_eq!(Kind::Job.info().path_in("shop"), "/apis/batch/v1/namespaces/shop/jobs");
    assert_eq!((Kind::Secret.info().tier, Kind::ConfigMap.info().tier), (Tier::B, Tier::B));
    assert!(Kind::Secret.info().namespaced && !Kind::ClusterRole.info().namespaced);
}

#[test]
fn the_first_lists_are_the_kinds_that_each_view_needs_and_the_events_come_last() {
    let stored: Vec<Kind> = Kind::ALL.iter().copied().filter(|kind| kind.info().tier != Tier::C).collect();
    assert_eq!(stored[..3], [Kind::Node, Kind::Namespace, Kind::Pod]);
    assert_eq!(stored.last(), Some(&Kind::Event));
    assert!(Kind::Pod < Kind::Secret && Kind::Secret < Kind::Event, "the sequence is the priority of a kind");
    assert_eq!(<Pod as Describe>::ID, Kind::Pod);
}

/// Plan section 10: each kind has the actions of each kind, and a kind adds its own ones
/// once.
#[test]
fn each_kind_has_the_actions_of_each_kind_and_its_own_ones_once() {
    for kind in Kind::ALL.iter().copied() {
        let actions: Vec<Action> = kind.actions().collect();
        let unique: BTreeSet<Action> = actions.iter().copied().collect();
        assert_eq!(unique.len(), actions.len(), "{kind:?} has an action two times");
        assert!(actions.starts_with(&Action::EACH_KIND), "{kind:?}");
    }
    let own = |kind: Kind| kind.info().actions.iter().map(|action| action.label()).collect::<Vec<_>>().join(", ");
    assert_eq!(own(Kind::Node), "Cordon, Uncordon, Drain, Node shell");
    assert_eq!(own(Kind::CronJob), "Trigger now, Suspend, Resume");
    assert!(!Kind::DaemonSet.info().actions.contains(&Action::Scale), "one pod on each node");
    let forward: Vec<Kind> =
        Kind::ALL.iter().copied().filter(|kind| kind.info().actions.contains(&Action::Forward)).collect();
    let targets = [
        Kind::Pod,
        Kind::Deployment,
        Kind::StatefulSet,
        Kind::DaemonSet,
        Kind::ReplicaSet,
        Kind::ReplicationController,
        Kind::Job,
        Kind::Service,
    ];
    assert_eq!(forward, targets, "plan section 12.3");
}

/// The visit gives the type of each kind, so a view of many kinds can read the columns
/// of each one.
#[test]
fn a_visit_gives_the_type_of_each_kind() {
    struct Titles;
    impl Visit for Titles {
        type Output = (Kind, Vec<&'static str>);
        fn visit<K: Describe>(self) -> Self::Output {
            (K::ID, columns::<K>().iter().map(|column| column.title).collect())
        }
    }
    for kind in Kind::ALL.iter().copied() {
        let (visited, titles) = kind.visit(Titles);
        assert_eq!(visited, kind);
        let unique: BTreeSet<&str> = titles.iter().copied().collect();
        assert_eq!(unique.len(), titles.len(), "{kind:?} has two columns with one title");
        assert_eq!((titles.first(), titles.last()), (Some(&"Name"), Some(&"Age")), "{kind:?}");
    }
    let (_, pod) = Kind::Pod.visit(Titles);
    assert_eq!(pod[..7], ["Name", "Namespace", "Ready", "Status", "Restarts", "Last restart", "IP"]);
}
