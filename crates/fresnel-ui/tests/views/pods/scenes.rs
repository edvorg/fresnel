//! The clusters of the view tests that more than one test uses.

use std::sync::Arc;

use fresnel_core::CoreError;
use fresnel_core::app::{ClusterRow, CoreView};
use fresnel_core::cluster::LinkState;
use fresnel_core::net::ErrorClass;
use fresnel_core::store::StoreState;
use k8s_openapi::api::core::v1::Pod;

use crate::data::{cluster, empty_store, live, pods, renamed, store};
use crate::fake::{SECOND, error, journal, now, off, row, view};

/// The pods of the scene with this text before each name and each UID: the pods of an
/// other cluster.
pub(crate) fn pods_of(prefix: &str) -> Vec<Arc<Pod>> {
    let copy = |pod: &Arc<Pod>| {
        let (name, uid) =
            (pod.metadata.name.as_deref().unwrap_or_default(), pod.metadata.uid.as_deref().unwrap_or_default());
        renamed(pod, &format!("{prefix}-{name}"), &format!("{prefix}-{uid}"))
    };
    pods().iter().filter(|pod| pod.metadata.namespace.as_deref() == Some("scene")).take(6).map(copy).collect()
}

/// A request that failed because the network gave no answer.
pub(crate) fn no_answer() -> CoreError {
    CoreError::new(ErrorClass::Timeout, "the list of the pods got no answer in 15s")
}

/// The answer of a server to a user who cannot list pods.
pub(crate) fn forbidden() -> CoreError {
    let text = "pods is forbidden: User \"viewer\" cannot list resource \"pods\" at the cluster scope (HTTP 403)";
    CoreError::new(ErrorClass::Forbidden, text)
}

/// A session with a fault in three of five clusters: a store in its pause with old rows, a
/// store that the user cannot read, and a connect that did not start.
pub(crate) fn session() -> CoreView {
    let pause = StoreState::Backoff { failures: 3, reason: no_answer(), until: now() + 2 * SECOND };
    let forbidden = StoreState::Forbidden { reason: forbidden() };
    let clusters: Vec<ClusterRow> = vec![
        live("ams-prod", &pods()),
        live("fra-stage", &pods_of("fra")),
        cluster("sgp-prod", store(pause, &pods_of("sgp"))),
        cluster("nyc-prod", empty_store(forbidden, now() - 20 * SECOND)),
        row("lab", LinkState::Failed(error("the cluster entry `lab` has no certificate authority"))),
        off("minikube"),
    ];
    let mut session = view(clusters);
    session.journal = journal(
        &[
            (fresnel_core::journal::Level::Info, "ams-prod", "live: a request got an answer on each channel"),
            (
                fresnel_core::journal::Level::Error,
                "lab",
                "the connect did not start: the cluster entry `lab` has no certificate authority",
            ),
            (
                fresnel_core::journal::Level::Warning,
                "sgp-prod",
                "a connection was lost (watches: the server sent no ping answer in 5s); it connects again",
            ),
        ],
        0,
    );
    session
}
