//! The `kubectl` commands of an object for "copy as kubectl command" (plan section 10): the
//! same object in the terminal of the user, in its own cluster.
//!
//! Each command names the context of the cluster, so it never reaches the current context
//! of the kubeconfig (design rule 8). Each word with a sign of the shell gets quotes.

use crate::cluster::ClusterId;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// The shell of an exec: bash when the image has it, else sh (research 08 section 6.2).
/// With `exec`, the exit code of the shell goes to the caller, and no second shell starts.
const SHELL: &str = "command -v bash >/dev/null 2>&1 && exec bash || exec sh";

/// `kubectl get` of the object as YAML.
pub fn get(cluster: &ClusterId, kind: Kind, key: &ObjectKey) -> String {
    let start = start(cluster, "get");
    format!("{start} {}{} {} --output yaml", resource(kind), namespace(key), quoted(&key.name))
}

/// `kubectl edit` of the object.
pub fn edit(cluster: &ClusterId, kind: Kind, key: &ObjectKey) -> String {
    format!("{} {}{} {}", start(cluster, "edit"), resource(kind), namespace(key), quoted(&key.name))
}

/// `kubectl logs` that follows a container of a pod, or of a pod that kubectl selects for
/// a workload. With no container, kubectl takes the default one of the pod.
pub fn logs(cluster: &ClusterId, kind: Kind, key: &ObjectKey, container: Option<&str>) -> String {
    let (start, object) = (start(cluster, "logs"), object(kind, key));
    format!("{start}{} {object}{} --follow", namespace(key), container_flag(container))
}

/// `kubectl exec` of an interactive shell in a container of a pod.
pub fn exec(cluster: &ClusterId, key: &ObjectKey, container: Option<&str>) -> String {
    let (start, pod, container) = (start(cluster, "exec"), quoted(&key.name), container_flag(container));
    format!("{start} --stdin --tty{} {pod}{container} -- sh -c {}", namespace(key), quoted(SHELL))
}

/// `kubectl port-forward` of a local port to a port of a pod, a Service or a workload.
pub fn port_forward(cluster: &ClusterId, kind: Kind, key: &ObjectKey, local: u16, remote: u16) -> String {
    format!("{}{} {} {local}:{remote}", start(cluster, "port-forward"), namespace(key), object(kind, key))
}

fn start(cluster: &ClusterId, verb: &str) -> String {
    format!("kubectl --context {} {verb}", quoted(cluster.as_str()))
}

/// The resource of the kind with its group, so a kind of an other group with the same
/// name does not match: `deployments.apps`.
fn resource(kind: Kind) -> String {
    let info = kind.info();
    if info.group.is_empty() { info.plural.to_owned() } else { format!("{}.{}", info.plural, info.group) }
}

/// The object as `<resource>/<name>`.
fn object(kind: Kind, key: &ObjectKey) -> String {
    quoted(&format!("{}/{}", resource(kind), key.name))
}

fn namespace(key: &ObjectKey) -> String {
    if key.namespace.is_empty() { String::new() } else { format!(" --namespace {}", quoted(&key.namespace)) }
}

fn container_flag(container: Option<&str>) -> String {
    container.map(|container| format!(" --container {}", quoted(container))).unwrap_or_default()
}

/// A word for the shell. A context name can have any character, for example the `:` and
/// the `/` of an ARN, so a word with a sign that the shell reads gets single quotes.
fn quoted(word: &str) -> String {
    let plain = |char: char| char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '.' | ':' | '/' | '@');
    if !word.is_empty() && word.chars().all(plain) {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(namespace: &str, name: &str) -> ObjectKey {
        ObjectKey::new(namespace, name)
    }

    #[test]
    fn get_and_edit_name_the_context_the_resource_with_its_group_the_namespace_and_the_name() {
        let pod = get(&ClusterId::new("ams-prod"), Kind::Pod, &key("shop", "web-1"));
        assert_eq!(pod, "kubectl --context ams-prod get pods --namespace shop web-1 --output yaml");
        let node = get(&ClusterId::new("do-ams3-prod"), Kind::Node, &key("", "pool-a1"));
        assert_eq!(node, "kubectl --context do-ams3-prod get nodes pool-a1 --output yaml");
        let deployment = edit(&ClusterId::new("a"), Kind::Deployment, &key("shop", "web"));
        assert_eq!(deployment, "kubectl --context a edit deployments.apps --namespace shop web");
    }

    #[test]
    fn logs_exec_and_port_forward_name_the_object_and_its_container() {
        let a = ClusterId::new("a");
        assert_eq!(
            logs(&a, Kind::Pod, &key("shop", "web-1"), Some("app")),
            "kubectl --context a logs --namespace shop pods/web-1 --container app --follow"
        );
        assert_eq!(
            logs(&a, Kind::Deployment, &key("shop", "web"), None),
            "kubectl --context a logs --namespace shop deployments.apps/web --follow"
        );
        let shell = "'command -v bash >/dev/null 2>&1 && exec bash || exec sh'";
        assert_eq!(
            exec(&a, &key("shop", "web-1"), Some("app")),
            format!("kubectl --context a exec --stdin --tty --namespace shop web-1 --container app -- sh -c {shell}")
        );
        assert_eq!(
            port_forward(&a, Kind::Service, &key("shop", "web"), 8080, 80),
            "kubectl --context a port-forward --namespace shop services/web 8080:80"
        );
    }

    #[test]
    fn a_word_with_a_sign_of_the_shell_gets_quotes() {
        assert_eq!(quoted("arn:aws:eks:eu-west-1:1:cluster/prod"), "arn:aws:eks:eu-west-1:1:cluster/prod");
        assert_eq!(quoted("my cluster"), "'my cluster'");
        assert_eq!(quoted("it's"), r"'it'\''s'");
        assert_eq!(quoted("$(reboot)"), "'$(reboot)'");
        assert_eq!(quoted(""), "''");
        let odd = logs(&ClusterId::new("my cluster"), Kind::Pod, &key("shop", "web"), Some("a b"));
        assert_eq!(odd, "kubectl --context 'my cluster' logs --namespace shop pods/web --container 'a b' --follow");
    }
}
