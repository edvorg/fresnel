//! The requests that each action sends, as verbs on resources (research 08 section 11.4).
//! An action is possible when the user can make each one.

use std::sync::Arc;

use super::Attributes;
use crate::kinds::{Action, Kind};
use crate::store::ObjectKey;

/// The requests of an action on an object. Empty for an action that reads only the stores
/// of the app or that sends nothing, such as "copy name". The verbs of the actions of
/// later milestones (the port forward) come with them. `node_shells` is the namespace of the
/// pod of a node shell, a setting.
pub fn needs(action: Action, kind: Kind, key: &ObjectKey, node_shells: &str) -> Vec<Attributes> {
    let at = |verb: &'static str, kind: Kind, subresource: &'static str, namespace: Arc<str>, name: &Arc<str>| {
        let info = kind.info();
        Attributes { verb, group: info.group, resource: info.plural, subresource, namespace, name: Arc::clone(name) }
    };
    let on = |verb, kind: Kind, subresource, name| {
        let namespace = if kind.info().namespaced { Arc::clone(&key.namespace) } else { "".into() };
        at(verb, kind, subresource, namespace, name)
    };
    let (this, none): (&Arc<str>, &Arc<str>) = (&key.name, &Arc::from(""));
    let revisions = if kind == Kind::Deployment { Kind::ReplicaSet } else { Kind::ControllerRevision };
    match action {
        Action::ViewManifest | Action::Download => vec![on("get", kind, "", this)],
        Action::EditManifest => vec![on("get", kind, "", this), on("update", kind, "", this)],
        Action::Delete | Action::ForceDelete => vec![on("delete", kind, "", this)],
        Action::Scale => vec![on("patch", kind, "scale", this)],
        Action::Restart | Action::Suspend | Action::Resume | Action::Cordon | Action::Uncordon => {
            vec![on("get", kind, "", this), on("patch", kind, "", this)]
        }
        Action::RevisionHistory => vec![on("get", kind, "", this), on("list", revisions, "", none)],
        Action::Rollback => {
            vec![on("get", kind, "", this), on("list", revisions, "", none), on("patch", kind, "", this)]
        }
        Action::Evict => vec![on("create", Kind::Pod, "eviction", this)],
        Action::RunAgain => {
            vec![on("get", Kind::Job, "", this), on("create", Kind::Job, "", none), on("delete", Kind::Job, "", this)]
        }
        Action::Trigger => vec![on("get", Kind::CronJob, "", this), on("create", Kind::Job, "", none)],
        // A drain reads and evicts the pods of each namespace: the key of a node has none.
        Action::Drain => vec![
            on("patch", kind, "", this),
            on("list", Kind::Pod, "", none),
            on("get", Kind::DaemonSet, "", none),
            on("create", Kind::Pod, "eviction", none),
        ],
        Action::EditKeys | Action::AddKey | Action::RemoveKey => {
            vec![on("get", kind, "", this), on("patch", kind, "", this)]
        }
        // A workload view reads the pods from the owner index: the log of each one.
        Action::Logs => vec![on("get", Kind::Pod, "log", if kind == Kind::Pod { this } else { none })],
        // A shell is an exec; the close of its tab ends it through a second one.
        Action::Terminal => vec![on("create", Kind::Pod, "exec", this)],
        Action::Attach => vec![on("create", Kind::Pod, "attach", this)],
        Action::DebugContainer => {
            vec![on("patch", Kind::Pod, "ephemeralcontainers", this), on("create", Kind::Pod, "attach", this)]
        }
        // A new pod of the node shell, an attach to it, and its delete at the end.
        Action::NodeShell => {
            let pods = |verb, subresource| at(verb, Kind::Pod, subresource, node_shells.into(), none);
            vec![pods("create", ""), pods("create", "attach"), pods("delete", "")]
        }
        Action::CopyName | Action::CopyCommand | Action::RolloutStatus | Action::Forward => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The namespace of the node shells in the settings of the tests.
    const NODE_SHELLS: &str = "tools";

    fn texts(action: Action, kind: Kind, key: &ObjectKey) -> Vec<String> {
        needs(action, kind, key, NODE_SHELLS).iter().map(ToString::to_string).collect()
    }

    #[test]
    fn each_operation_needs_the_verbs_of_its_requests() {
        let web = ObjectKey::new("shop", "web");
        assert_eq!(texts(Action::Scale, Kind::Deployment, &web), ["patch deployments/scale (apps) web in shop"]);
        assert_eq!(texts(Action::Delete, Kind::Pod, &web), ["delete pods web in shop"]);
        assert_eq!(texts(Action::Evict, Kind::Pod, &web), ["create pods/eviction web in shop"]);
        assert_eq!(
            texts(Action::Rollback, Kind::StatefulSet, &web),
            [
                "get statefulsets (apps) web in shop",
                "list controllerrevisions (apps) in shop",
                "patch statefulsets (apps) web in shop"
            ]
        );
        assert_eq!(texts(Action::RevisionHistory, Kind::Deployment, &web)[1], "list replicasets (apps) in shop");
        assert_eq!(
            texts(Action::Trigger, Kind::CronJob, &web),
            ["get cronjobs (batch) web in shop", "create jobs (batch) in shop"]
        );
        assert_eq!(texts(Action::RunAgain, Kind::Job, &web).len(), 3);
        let node = ObjectKey::new("", "pool-a1");
        assert_eq!(texts(Action::Delete, Kind::Node, &node), ["delete nodes pool-a1 in the cluster"]);
        let drain =
            ["patch nodes pool-a1 in the cluster", "list pods in the cluster", "get daemonsets (apps) in the cluster"];
        assert_eq!(texts(Action::Drain, Kind::Node, &node)[..3], drain);
        assert_eq!(texts(Action::Drain, Kind::Node, &node)[3], "create pods/eviction in the cluster");
        assert!(texts(Action::RolloutStatus, Kind::Deployment, &web).is_empty(), "it reads the store");
        assert!(texts(Action::CopyName, Kind::Pod, &web).is_empty());
        assert_eq!(texts(Action::Logs, Kind::Pod, &web), ["get pods/log web in shop"]);
        assert_eq!(texts(Action::Logs, Kind::Deployment, &web), ["get pods/log in shop"], "each pod of the workload");
    }

    /// A terminal needs the verbs of each request of its session (plan section 12.8), and a
    /// node shell those of its pod in the namespace of the settings.
    #[test]
    fn a_terminal_needs_its_exec_or_attach_and_a_node_shell_its_pod() {
        let web = ObjectKey::new("shop", "web");
        assert_eq!(texts(Action::Terminal, Kind::Pod, &web), ["create pods/exec web in shop"]);
        assert_eq!(texts(Action::Attach, Kind::Pod, &web), ["create pods/attach web in shop"]);
        let debug = ["patch pods/ephemeralcontainers web in shop", "create pods/attach web in shop"];
        assert_eq!(texts(Action::DebugContainer, Kind::Pod, &web), debug);
        let node = ObjectKey::new("", "pool-a1");
        let pod = ["create pods in tools", "create pods/attach in tools", "delete pods in tools"];
        assert_eq!(texts(Action::NodeShell, Kind::Node, &node), pod);
    }
}
