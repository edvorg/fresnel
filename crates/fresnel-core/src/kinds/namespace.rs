//! The Namespace: its health value.

use k8s_openapi::api::core::v1::Namespace;

use super::health::{Assess, ObjectHealth, is_true};

/// The conditions that tell why the delete of a namespace cannot continue.
const DELETE_FAILURES: [&str; 3] = [
    "NamespaceDeletionDiscoveryFailure",
    "NamespaceDeletionGroupVersionParsingFailure",
    "NamespaceDeletionContentFailure",
];

impl Assess for Namespace {
    /// A namespace in its delete is on its way. Content that waits for a finalizer is a
    /// part of that way. The delete failed when the server cannot find or remove the content.
    fn health(&self) -> ObjectHealth {
        let status = self.status.as_ref();
        let conditions = status.and_then(|status| status.conditions.as_ref());
        if DELETE_FAILURES.iter().any(|failure| is_true(conditions, failure)) {
            ObjectHealth::Failed
        } else if status.and_then(|status| status.phase.as_deref()) == Some("Terminating") {
            ObjectHealth::Progressing
        } else {
            ObjectHealth::Healthy
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{self, condition};
    use crate::kinds::ObjectHealth::{Failed, Healthy, Progressing};

    #[test]
    fn a_namespace_in_its_delete_is_on_its_way_until_the_server_cannot_remove_its_content() {
        assert_eq!(fixture::one::<Namespace>("scene").health(), Healthy);
        // A finalizer holds a ConfigMap of this namespace: the delete waits, and did not fail.
        assert_eq!(fixture::one::<Namespace>("scene-ending").health(), Progressing);
        let stuck = fixture::changed::<Namespace>("scene-ending", |namespace| {
            condition(namespace, "NamespaceDeletionContentFailure")["status"] = "True".into();
        });
        assert_eq!(stuck.health(), Failed);
    }
}
