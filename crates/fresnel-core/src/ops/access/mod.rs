//! The permissions of the user in a cluster (plan section 10, research 08 section 11).
//!
//! One SelfSubjectRulesReview gives the rules of the user in one namespace; the app asks
//! for each namespace that a view opens. The rules decide the state of each action. For a
//! kind with no namespace, and when the server gives only a part of the rules, one
//! SelfSubjectAccessReview gives the exact answer for one action. The rules are a hint
//! for the UI only: the app also handles HTTP 403 of the real request.
//!
//! - `rules`: the match of a rule, with `*/<subresource>` and `resourceNames`.
//! - `review`: the two review requests and their answers. `needs`: the verbs of each action.

mod needs;
mod review;
mod rules;

use std::fmt::{self, Display};
use std::sync::Arc;

pub use needs::needs;
pub use review::{check, read_rules};
pub use rules::Rules;

/// One request that the API server authorizes: a verb on a resource.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Attributes {
    /// For example `patch`.
    pub verb: &'static str,
    /// The API group. Empty for the core group.
    pub group: &'static str,
    /// The resource, for example `deployments`.
    pub resource: &'static str,
    /// The subresource, for example `scale`, or empty.
    pub subresource: &'static str,
    /// The namespace. Empty for a kind with no namespace.
    pub namespace: Arc<str>,
    /// The name of the object, or empty for each object, as for a create or a list.
    pub name: Arc<str>,
}

impl Display for Attributes {
    /// "patch deployments/scale in shop": the words of a reason.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.verb, self.resource)?;
        if !self.subresource.is_empty() {
            write!(f, "/{}", self.subresource)?;
        }
        if !self.group.is_empty() {
            write!(f, " ({})", self.group)?;
        }
        if !self.name.is_empty() {
            write!(f, " {}", self.name)?;
        }
        match &*self.namespace {
            "" => f.write_str(" in the cluster"),
            namespace => write!(f, " in {namespace}"),
        }
    }
}

/// What the app knows about one action of the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// A rule permits it.
    Allowed,
    /// No rule permits it. The text gives the reason for a disabled action.
    Denied(String),
    /// The rules do not tell; an access review asks the server. The text tells why.
    Unknown(String),
}

impl Decision {
    /// The decision for many requests: each one must be allowed.
    pub fn all(decisions: impl IntoIterator<Item = Decision>) -> Decision {
        let mut unknown = None;
        for decision in decisions {
            match decision {
                Decision::Allowed => {}
                Decision::Denied(reason) => return Decision::Denied(reason),
                Decision::Unknown(reason) => unknown = unknown.or(Some(reason)),
            }
        }
        unknown.map_or(Decision::Allowed, Decision::Unknown)
    }
}
