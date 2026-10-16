//! The containers that the app makes for a terminal: a debug container in a pod, and the pod
//! of a node shell. The close of the tab ends the first and deletes the second.

use fresnel_core::cluster::ClusterId;
use fresnel_core::store::ObjectKey;
use fresnel_core::terminal::{Program, TerminalRequest};

use crate::bed::{Bed, CLUSTER, eventually};
use crate::screen::keys;
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
    let node = bed.kubectl(&["get", "nodes", "-o", "jsonpath={.items[0].metadata.name}"]).await;
    let uid = bed.kubectl(&["get", "node", &node, "-o", "jsonpath={.metadata.uid}"]).await;
    let request = TerminalRequest {
        cluster: ClusterId::new(CLUSTER),
        object: ObjectKey::new("", node.as_str()),
        uid: uid.as_str().into(),
        container: "".into(),
        program: Program::NodeShell,
    };
    let view = bed.open(request).await;
    keys(&view.terminal, "hostname; echo node-$((7*8))\r");
    let rows = bed.screen(&view.terminal, "node-56").await;
    assert!(rows.iter().any(|row| row.trim() == node), "the host name of the node: {rows:#?}");
    let pods = bed.kubectl(&[&NODE_SHELLS[..], &["-o", "name"]].concat()).await;
    assert_eq!(pods.lines().count(), 1, "one pod of the node shell: {pods}");
    close(bed, &view).await;
    eventually("the pod of the node shell is gone", || async {
        bed.kubectl(&[&NODE_SHELLS[..], &["-o", "name"]].concat()).await.trim().is_empty()
    })
    .await;
}
