//! The table `[forwards]`: the settings of the group, and the saved rules in `saved`.

use std::collections::BTreeMap;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::{Reader, integer};
use crate::cluster::ClusterId;
use crate::config::forwards::{RULE_KEYS, SAVED};
use crate::config::table::{Group, Setting};
use crate::config::values::Value;
use crate::forward::{Bind, ForwardRule, LocalPort, TARGETS};
use crate::store::ObjectKey;

/// The most saved rules: more than one window shows.
const RULE_LIMIT: usize = 256;

impl Reader<'_> {
    /// The table `[forwards]`. Gives the saved rules, in the sequence of their keys.
    pub(super) fn forwards(
        &mut self,
        value: &Spanned<DeValue<'_>>,
        values: &mut BTreeMap<Setting, Value>,
    ) -> Vec<ForwardRule> {
        let Some(table) = self.table_of(Group::Forwards.name(), value) else {
            return Vec::new();
        };
        let mut rules = Vec::new();
        for (key, value) in table.iter() {
            match key.get_ref().as_ref() {
                SAVED => rules = self.saved(value),
                _ => self.fixed_entry(Group::Forwards, key, value, values),
            }
        }
        rules
    }

    fn saved(&mut self, value: &Spanned<DeValue<'_>>) -> Vec<ForwardRule> {
        let Some(table) = self.table_of("forwards.saved", value) else {
            return Vec::new();
        };
        let mut rules = Vec::new();
        for (name, value) in table.iter() {
            let path = format!("forwards.saved.\"{}\"", name.get_ref());
            if rules.len() == RULE_LIMIT {
                self.problem(name.span(), format!("`{path}`: the app keeps {RULE_LIMIT} saved forwards at most"));
                break;
            }
            let Some(entries) = self.table_of(&path, value) else {
                continue;
            };
            if let Some(rule) = self.rule(&path, entries, value) {
                rules.push(rule);
            }
        }
        rules
    }

    /// One saved rule. A key that does not fit is a problem, and the rule is not used.
    fn rule(&mut self, path: &str, entries: &DeTable<'_>, table: &Spanned<DeValue<'_>>) -> Option<ForwardRule> {
        let mut found: BTreeMap<&str, &Spanned<DeValue<'_>>> = BTreeMap::new();
        for (key, value) in entries.iter() {
            match RULE_KEYS.iter().find(|known| **known == key.get_ref().as_ref()) {
                Some(known) => {
                    found.insert(known, value);
                }
                None => {
                    let (name, keys) = (key.get_ref(), RULE_KEYS.join(", "));
                    self.problem(key.span(), format!("unknown key `{name}` of a saved forward; the keys are {keys}"));
                }
            }
        }
        if let Some(missing) = RULE_KEYS.iter().find(|key| !found.contains_key(*key)) {
            self.problem(table.span(), format!("`{path}` has no `{missing}`; the app does not use it"));
            return None;
        }
        let get = |key: &str| found.get(key).copied();
        let text = |reader: &mut Self, key: &str| reader.text(&format!("{path}.{key}"), get(key)?).map(str::to_owned);
        let (cluster, namespace, name) = (text(self, "cluster"), text(self, "namespace"), text(self, "name"));
        let kind = text(self, "kind").and_then(|kind| {
            let found = TARGETS.iter().copied().find(|target| target.info().name == kind);
            if found.is_none() {
                let names: Vec<&str> = TARGETS.iter().map(|target| target.info().name).collect();
                let text = format!("`{path}.kind` = \"{kind}\" is no kind of a forward: {}", names.join(", "));
                self.problem(get("kind").map_or(0..0, Spanned::span), text);
            }
            found
        });
        let bind = text(self, "bind").and_then(|bind| {
            let found = Bind::parse(&bind);
            if found.is_none() {
                let text = format!("`{path}.bind` = \"{bind}\" is \"localhost\", \"everywhere\" or an address");
                self.problem(get("bind").map_or(0..0, Spanned::span), text);
            }
            found
        });
        let port = self.port(path, "port", get("port")?, 1);
        let local = self.port(path, "local_port", get("local_port")?, 0).map(|port| match port {
            0 => LocalPort::Automatic,
            port => LocalPort::Fixed(port),
        });
        let rule = ForwardRule {
            cluster: ClusterId::new(cluster?),
            kind: kind?,
            key: ObjectKey::new(&namespace?, &name?),
            port: port?,
            local: local?,
            bind: bind?,
        };
        if let Some(problem) = rule.problem() {
            self.problem(table.span(), format!("`{path}`: {problem}"));
            return None;
        }
        Some(rule)
    }

    /// A port of a saved rule, from `lowest` to 65535.
    fn port(&mut self, path: &str, key: &str, value: &Spanned<DeValue<'_>>, lowest: u16) -> Option<u16> {
        let port = integer(value).and_then(|number| u16::try_from(number).ok()).filter(|port| *port >= lowest);
        if port.is_none() {
            self.problem(value.span(), format!("`{path}.{key}` is a whole number from {lowest} to 65535"));
        }
        port
    }
}
