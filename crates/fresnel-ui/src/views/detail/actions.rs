//! The actions on the object of the panel that this version runs: the Manifest tab, the copy
//! of the name and the copy of a `kubectl` command. The command table has each other
//! action too; a later milestone gives it its request, and its menu entry is off until then.

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::{Action, Kind};
use fresnel_core::store::ObjectKey;

/// True for an action that this version runs.
pub(crate) fn can_run(action: Action) -> bool {
    matches!(action, Action::ViewManifest | Action::CopyName | Action::CopyCommand)
}

/// Why an action does nothing in this version.
pub(crate) fn later(action: Action) -> String {
    format!("{} comes with a later milestone of the app.", action.label())
}

/// The `kubectl` command that prints the object, for its cluster: the same object in the
/// terminal of the user.
pub(crate) fn kubectl(cluster: &ClusterId, kind: Kind, key: &ObjectKey) -> String {
    let info = kind.info();
    let resource =
        if info.group.is_empty() { info.plural.to_owned() } else { format!("{}.{}", info.plural, info.group) };
    let namespace =
        if key.namespace.is_empty() { String::new() } else { format!(" --namespace {}", quoted(&key.namespace)) };
    let (context, name) = (quoted(cluster.as_str()), quoted(&key.name));
    format!("kubectl --context {context} get {resource}{namespace} {name} --output yaml")
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

    #[test]
    fn the_command_names_the_context_the_resource_with_its_group_the_namespace_and_the_name() {
        let pod = kubectl(&ClusterId::new("ams-prod"), Kind::Pod, &ObjectKey::new("shop", "web-1"));
        assert_eq!(pod, "kubectl --context ams-prod get pods --namespace shop web-1 --output yaml");
        let node = kubectl(&ClusterId::new("do-ams3-prod"), Kind::Node, &ObjectKey::new("", "pool-a1"));
        assert_eq!(node, "kubectl --context do-ams3-prod get nodes pool-a1 --output yaml");
        let deployment = kubectl(&ClusterId::new("a"), Kind::Deployment, &ObjectKey::new("shop", "web"));
        assert_eq!(deployment, "kubectl --context a get deployments.apps --namespace shop web --output yaml");
    }

    #[test]
    fn a_word_with_a_sign_of_the_shell_gets_quotes() {
        assert_eq!(quoted("arn:aws:eks:eu-west-1:1:cluster/prod"), "arn:aws:eks:eu-west-1:1:cluster/prod");
        assert_eq!(quoted("my cluster"), "'my cluster'");
        assert_eq!(quoted("it's"), r"'it'\''s'");
        assert_eq!(quoted("$(reboot)"), "'$(reboot)'");
        assert_eq!(quoted(""), "''");
    }
}
