//! The detail sections of the kinds of the Access control group (plan section 11.3):
//! ServiceAccount, Role, ClusterRole, RoleBinding and ClusterRoleBinding. The content of each
//! one is the union of Headlamp and Lens (`research/01` section 3.4.4, `research/02` section
//! 4.6), with the rules in the words of `kubectl describe`.
//!
//! - `rules`: the rules of a Role and of a ClusterRole.
//! - `bindings`: the role and the subjects of a binding, as links.
//! - `account`: a ServiceAccount, its Secrets and the bindings that name it.

pub(crate) mod account;
pub(crate) mod bindings;
pub(crate) mod rules;
