//! The permissions of the user for the actions that a view shows (plan section 10,
//! `research/08` section 11.3).
//!
//! The core keeps the rules of each namespace and the answers of access reviews; a view
//! asks for what it needs. A view asks one time for each question, until the core has an
//! entry for it: so a question goes again after a refusal with HTTP 403 drops the rules,
//! and a failed read goes again one time for each new holder, not in each frame.

use std::collections::HashSet;
use std::sync::Arc;

use fresnel_core::app::{AccessView, CheckState, Command, CoreView, RulesState};
use fresnel_core::cluster::ClusterId;
use fresnel_core::config::table::Setting;
use fresnel_core::kinds::Action;
use fresnel_core::ops::Target;
use fresnel_core::ops::access::{self, Attributes, Decision};

/// What one holder (a panel, a menu, a dialog) asked of the core.
#[derive(Debug, Default)]
pub(crate) struct Permits {
    rules: HashSet<(ClusterId, Arc<str>)>,
    checks: HashSet<(ClusterId, Attributes)>,
}

impl Permits {
    /// The decision for an action on an object, and the commands that ask the core for
    /// what it does not know yet.
    pub(crate) fn decide(
        &mut self,
        view: &CoreView,
        target: &Target,
        action: Action,
        commands: &mut Vec<Command>,
    ) -> Decision {
        let row = view.clusters.iter().find(|row| row.id == target.cluster);
        let Some(access) = row.and_then(|row| row.data.as_ref()).map(|data| &data.access) else {
            return Decision::Unknown(format!("{} is not connected", target.cluster));
        };
        let node_shells = view.config.text(Setting::NodeShellNamespace);
        let requests = access::needs(action, target.kind, &target.key, node_shells);
        for attributes in &requests {
            self.ask(&target.cluster, access, attributes.clone(), commands);
        }
        access.decide(&requests)
    }

    /// Asks for the rules of the namespace of one request, or for an access review when
    /// the rules cannot tell: for a kind with no namespace, and for rules that are not complete.
    fn ask(&mut self, cluster: &ClusterId, access: &AccessView, attributes: Attributes, commands: &mut Vec<Command>) {
        let exact = match access.checks.get(&attributes) {
            Some(CheckState::Answered(_)) => {
                self.checks.remove(&(cluster.clone(), attributes));
                return;
            }
            Some(CheckState::Asking) => return,
            Some(CheckState::Failed(_)) => true,
            None if attributes.namespace.is_empty() => true,
            None => {
                let namespace = Arc::clone(&attributes.namespace);
                match access.rules.get(&namespace) {
                    Some(RulesState::Read(rules)) => {
                        self.rules.remove(&(cluster.clone(), namespace));
                        matches!(rules.decide(&attributes), Decision::Unknown(_))
                    }
                    Some(RulesState::Reading) => false,
                    Some(RulesState::Failed(_)) | None => {
                        if self.rules.insert((cluster.clone(), Arc::clone(&namespace))) {
                            commands.push(Command::ReadRules { cluster: cluster.clone(), namespace });
                        }
                        false
                    }
                }
            }
        };
        if exact && self.checks.insert((cluster.clone(), attributes.clone())) {
            commands.push(Command::CheckAccess { cluster: cluster.clone(), attributes });
        }
    }
}
