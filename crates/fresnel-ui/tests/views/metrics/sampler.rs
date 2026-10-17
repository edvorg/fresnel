//! The sampler of each cluster in the Connections view: the state of each list with its
//! reason and the time of its last answer, the next poll and the size of the history.

use fresnel_core::cluster::ClusterId;
use fresnel_core::metrics::{Availability, Metrics, PollState};

use super::{failed, of_pods, scene_metrics};
use crate::fake::{SECOND, absent, connections_window, error, now, view};

#[test]
fn each_cluster_shows_the_state_of_each_list_of_its_sampler_with_its_reason() {
    let condition = Availability::Condition {
        status: "False".into(),
        reason: Some("FailedDiscoveryCheck".into()),
        message: Some("failing or missing response from https://10.43.0.9:10250".into()),
    };
    let unavailable = PollState::Unavailable(error("service unavailable (HTTP 503)"), Some(condition));
    let mut never = Metrics::waiting(ClusterId::new("lon"), 15 * SECOND, now() - 60 * SECOND);
    never.nodes.state = PollState::Forbidden(error("nodes.metrics.k8s.io is forbidden"));
    // A cluster with no metrics API gets a poll each 5 minutes only.
    let mut no_api = Metrics::waiting(ClusterId::new("sgp"), 15 * SECOND, now() - 60 * SECOND);
    no_api.pods.state = PollState::NotInstalled(error("the server could not find the requested resource"));
    no_api.nodes.state = no_api.pods.state.clone();
    let clusters = vec![
        of_pods("ams", scene_metrics("ams", 1)),
        of_pods("fra", failed(scene_metrics("fra", 1), unavailable, now() - 40 * SECOND)),
        of_pods("lon", never),
        of_pods("sgp", no_api),
    ];
    let window = connections_window(view(clusters));
    let texts = [
        "ok, last answer 5 s ago",
        "metrics-server not available: service unavailable (HTTP 503); the APIService v1beta1.metrics.k8s.io has \
         Available=False (FailedDiscoveryCheck): failing or missing response from https://10.43.0.9:10250 since \
         07:59:20; last answer 5 s ago",
        "waits for its first answer",
        "forbidden: nodes.metrics.k8s.io is forbidden since 07:59:00; no answer yet",
        "A poll each 15 s, next in 10 s. History: 30 series, 315Ki.",
        "A poll each 15 s. History: 0 series, 0.",
        "A poll each 5 min 0 s. History: 0 series, 0.",
    ];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
}
