//! The saved forwards of the settings file (plan section 12.3): one table for each rule in
//! `[forwards.saved]`. The forwards view saves and forgets a rule with the edits of this
//! module, and the setting `start_saved` of `[forwards]` starts each saved rule with the app.
//!
//! ```toml
//! [forwards.saved."ams-prod/shop/service/web:80"]
//! cluster = "ams-prod"
//! namespace = "shop"
//! kind = "Service"
//! name = "web"
//! port = 80
//! local_port = 8080     # 0: a free port of the system
//! bind = "localhost"    # "localhost", "everywhere" or an address
//! ```

use super::edit::{Edit, EditValue};
use crate::forward::{ForwardRule, LocalPort};

/// The table of the saved rules in `[forwards]`.
pub const SAVED: &str = "saved";
/// The keys of a saved rule, in the sequence of the file.
pub(super) const RULE_KEYS: [&str; 7] = ["cluster", "namespace", "kind", "name", "port", "local_port", "bind"];

/// The name of the table of a rule: its cluster and its target. A second rule of the same
/// target replaces the first one in the file.
pub fn saved_key(rule: &ForwardRule) -> String {
    let kind = rule.kind.info().name.to_lowercase();
    format!("{}/{}/{kind}/{}:{}", rule.cluster, rule.key.namespace, rule.key.name, rule.port)
}

/// The edits that save a rule in the settings file.
pub fn save_edits(rule: &ForwardRule) -> Vec<Edit> {
    let key = saved_key(rule);
    let edit = |field: &str, value| Edit::new(&["forwards", SAVED, &key, field], value);
    let text = |field: &str, value: &str| edit(field, EditValue::Text(value.to_owned()));
    let local = match rule.local {
        LocalPort::Fixed(port) => i64::from(port),
        LocalPort::Automatic => 0,
    };
    vec![
        text("cluster", rule.cluster.as_str()),
        text("namespace", &rule.key.namespace),
        text("kind", rule.kind.info().name),
        text("name", &rule.key.name),
        edit("port", EditValue::Number(i64::from(rule.port))),
        edit("local_port", EditValue::Number(local)),
        text("bind", &rule.bind.text()),
    ]
}

/// The edit that removes a saved rule from the settings file.
pub fn forget_edit(rule: &ForwardRule) -> Edit {
    Edit::new(&["forwards", SAVED, &saved_key(rule)], EditValue::Remove)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::*;
    use crate::cluster::ClusterId;
    use crate::config::edit::apply;
    use crate::config::parse;
    use crate::forward::Bind;
    use crate::kinds::Kind;
    use crate::store::ObjectKey;

    fn rule(local: LocalPort, bind: Bind) -> ForwardRule {
        ForwardRule {
            cluster: ClusterId::new("ams-prod"),
            kind: Kind::Service,
            key: ObjectKey::new("shop", "web"),
            port: 80,
            local,
            bind,
        }
    }

    #[test]
    fn a_saved_rule_reads_back_equal_and_a_forget_removes_its_table_only() {
        let text = "version = 1\n# my forwards\n[forwards]\nstart_saved = false\n";
        let first = rule(LocalPort::Fixed(8080), Bind::Localhost);
        let second =
            ForwardRule { port: 443, ..rule(LocalPort::Automatic, Bind::Address(Ipv4Addr::new(10, 0, 0, 7).into())) };
        let saved = apply(text, &[save_edits(&first), save_edits(&second)].concat()).expect("a save");
        assert!(saved.contains("[forwards.saved.\"ams-prod/shop/service/web:80\"]"), "{saved}");
        assert!(saved.starts_with(text), "the comments and the keys stay: {saved}");
        let loaded = parse(&saved);
        assert_eq!(loaded.problems, Vec::new());
        assert_eq!(loaded.config.forwards, [second.clone(), first.clone()], "in the sequence of their keys");

        let forgotten = apply(&saved, &[forget_edit(&first)]).expect("a forget");
        assert_eq!(parse(&forgotten).config.forwards, std::slice::from_ref(&second));
        let everywhere = rule(LocalPort::Fixed(9090), Bind::Everywhere);
        let replaced = apply(&saved, &save_edits(&everywhere)).expect("a save of the same target");
        assert_eq!(parse(&replaced).config.forwards, [second, everywhere.clone()], "one table for each target");
        assert!(matches!(everywhere.bind.addresses().first(), Some(IpAddr::V4(address)) if address.is_unspecified()));
    }

    #[test]
    fn a_saved_rule_that_does_not_fit_is_a_problem_at_its_place_and_is_not_used() {
        let text = "version = 1\n[forwards]\nwait_secs = 0\nstart = true\n\
                    [forwards.saved.a]\ncluster = \"k\"\nnamespace = \"s\"\nkind = \"ConfigMap\"\nname = \"c\"\n\
                    port = 80\nlocal_port = 0\nbind = \"nowhere\"\nproto = \"udp\"\n\
                    [forwards.saved.b]\ncluster = \"k\"\nnamespace = \"s\"\nkind = \"Pod\"\nname = \"p\"\nport = 70000\n";
        let loaded = parse(text);
        let texts: Vec<String> = loaded.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "line 3, column 13: `forwards.wait_secs` = 0 is refused: a period or a time limit of 0 never ends its wait; the app keeps 5 s",
                "line 4, column 1: unknown key `start` in `forwards`; the app ignores it",
                "line 8, column 8: `forwards.saved.\"a\".kind` = \"ConfigMap\" is no kind of a forward: Pod, Service, Deployment, StatefulSet, DaemonSet, ReplicaSet, ReplicationController, Job",
                "line 12, column 8: `forwards.saved.\"a\".bind` = \"nowhere\" is \"localhost\", \"everywhere\" or an address",
                "line 13, column 1: unknown key `proto` of a saved forward; the keys are cluster, namespace, kind, name, port, local_port, bind",
                "line 14, column 1: `forwards.saved.\"b\"` has no `local_port`; the app does not use it",
            ]
        );
        assert!(loaded.used && loaded.config.forwards.is_empty());
    }
}
