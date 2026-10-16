//! The containers that the app makes for a terminal: a debug container in a pod, and the pod
//! of a node shell. The close of the tab ends the first and deletes the second, also when the
//! server refused the attach to a debug container.

use fresnel_core::app::Command;
use fresnel_core::cluster::ClusterId;
use fresnel_core::store::ObjectKey;
use fresnel_core::terminal::{Program, SessionState, TerminalRequest};
use kube::core::Request;
use kube::core::params::PostParams;
use serde_json::{Value, json};

use crate::bed::{Bed, CLUSTER, eventually};
use crate::screen::{keys, processes};
use crate::shell::close;

/// The pods of node shells, by the label of the app.
const NODE_SHELLS: [&str; 6] = ["get", "pods", "-n", "kube-system", "-l", "app.kubernetes.io/component=node-shell"];

/// A debug container shares the process namespace of its target, so its `ps` shows the
/// `sleep` of the container `shell`. Ctrl-D ends its shell, and the container ends with it.
pub(crate) async fn a_debug_container_sees_the_processes_of_its_target_and_ends_with_its_tab(bed: &Bed) {
    let view = bed.open(bed.request(Program::Debug, "shell", "shell").await).await;
    bed.screen(&view.terminal, "/ #").await;
    keys(&view.terminal, "ps -o pid,args\r");
    // The debug container shares the process namespace of the target: it sees its PID 1.
    bed.screen(&view.terminal, "while true").await;
    close(bed, &view).await;
    let state = "jsonpath={.status.ephemeralContainerStatuses[*].state.terminated.reason}";
    eventually("the debug container ended", || async {
        !bed.kubectl(&["get", "pod", "shell", "-o", state]).await.trim().is_empty()
    })
    .await;
}

/// A node shell runs in the namespaces of the node: its host name is the name of the node.
/// Its pod has the label of the app, and the close of the tab deletes it.
pub(crate) async fn a_node_shell_runs_on_the_node_and_its_pod_goes_with_its_tab(bed: &Bed) {
    let request = node_shell(bed).await;
    let view = bed.open(request.clone()).await;
    keys(&view.terminal, "hostname; echo node-$((7*8))\r");
    let rows = bed.screen(&view.terminal, "node-56").await;
    assert!(rows.iter().any(|row| row.trim() == &*request.object.name), "the host name of the node: {rows:#?}");
    let pods = node_shells(bed).await;
    assert_eq!(pods.lines().count(), 1, "one pod of the node shell: {pods}");
    close(bed, &view).await;
    eventually("the pod of the node shell is gone", || async { node_shells(bed).await.trim().is_empty() }).await;
}

/// The request of a node shell on the first node.
pub(crate) async fn node_shell(bed: &Bed) -> TerminalRequest {
    let node = bed.kubectl(&["get", "nodes", "-o", "jsonpath={.items[0].metadata.name}"]).await;
    let uid = bed.kubectl(&["get", "node", &node, "-o", "jsonpath={.metadata.uid}"]).await;
    TerminalRequest {
        cluster: ClusterId::new(CLUSTER),
        object: ObjectKey::new("", node.as_str()),
        uid: uid.as_str().into(),
        container: "".into(),
        program: Program::NodeShell,
    }
}

/// The pods of node shells, by the label of the app: one name in each line.
pub(crate) async fn node_shells(bed: &Bed) -> String {
    bed.kubectl(&[&NODE_SHELLS[..], &["-o", "name"]].concat()).await
}

/// The policy that refuses each attach while it exists (a ValidatingAdmissionPolicy of the
/// operation CONNECT), and each new ConfigMap, which shows that the server applies it.
const REFUSAL: &str = "fresnel-test-no-attach";
const POLICIES: &str = "/apis/admissionregistration.k8s.io/v1/validatingadmissionpolicies";
const BINDINGS: &str = "/apis/admissionregistration.k8s.io/v1/validatingadmissionpolicybindings";

/// The server refuses the attach to a new debug container, whose shell then runs with no
/// stream: the line tells it, and the close of the tab ends it with the hangup.
pub(crate) async fn a_debug_container_whose_attach_fails_ends_its_shell_with_its_tab(bed: &Bed) {
    refuse_attaches(bed, true).await;
    let request = bed.request(Program::Debug, "shell", "shell").await;
    let before = bed.core.view().terminals.iter().map(|view| view.id).max().unwrap_or(0);
    bed.core.send(Command::OpenTerminal(request));
    let view = bed.until("the new terminal", |view| view.terminals.iter().any(|view| view.id > before)).await;
    let view = view.terminals.iter().find(|view| view.id > before).cloned().expect("the new terminal");
    let terminal = view.terminal.clone();
    let note = "the shell can still run in the pod: a close of the tab ends it";
    let failed = |state: &SessionState| matches!(state, SessionState::Failed { reason } if reason.ends_with(note));
    bed.until("the failed attach with its note", |_| failed(&terminal.state())).await;
    refuse_attaches(bed, false).await;
    let debug_shells = || async {
        let running = processes(bed, "shell", "shell").await;
        running.iter().filter(|line| !line.contains("while true") && !line.ends_with("sleep 1")).count()
    };
    assert_eq!(debug_shells().await, 1, "the shell of the debug container waits for input");
    close(bed, &view).await;
    eventually("no shell of a debug container", || async { debug_shells().await == 0 }).await;
}

/// Creates or deletes the policy that refuses each attach, and waits until the server applies
/// the change: a dry run of a new ConfigMap fails exactly while the policy is there.
async fn refuse_attaches(bed: &Bed, refuse: bool) {
    if refuse {
        let rule = |operation: &str, resource: &str| json!({"apiGroups": [""], "apiVersions": ["v1"], "operations": [operation], "resources": [resource]});
        let policy = json!({
            "apiVersion": "admissionregistration.k8s.io/v1", "kind": "ValidatingAdmissionPolicy",
            "metadata": {"name": REFUSAL},
            "spec": {
                "failurePolicy": "Fail",
                "matchConstraints": {"resourceRules": [rule("CONNECT", "pods/attach"), rule("CREATE", "configmaps")]},
                "validations": [{"expression": "false", "message": "the test refuses it"}],
            },
        });
        let binding = json!({
            "apiVersion": "admissionregistration.k8s.io/v1", "kind": "ValidatingAdmissionPolicyBinding",
            "metadata": {"name": REFUSAL},
            "spec": {"policyName": REFUSAL, "validationActions": ["Deny"]},
        });
        for (path, object) in [(POLICIES, policy), (BINDINGS, binding)] {
            let body = serde_json::to_vec(&object).expect("JSON");
            let create = Request::new(path).create(&PostParams::default(), body).expect("a request");
            let _: Value = bed.api.gateway.send(create).await.unwrap_or_else(|error| panic!("{path}: {error}"));
        }
    } else {
        bed.kubectl(&["delete", "validatingadmissionpolicybinding", REFUSAL]).await;
        bed.kubectl(&["delete", "validatingadmissionpolicy", REFUSAL]).await;
    }
    let probe = ["create", "configmap", "fresnel-probe", "--dry-run=server", "--output", "name"];
    eventually("the server applies the policy", || async { bed.k3s.kubectl(&probe).await.is_err() == refuse }).await;
}
