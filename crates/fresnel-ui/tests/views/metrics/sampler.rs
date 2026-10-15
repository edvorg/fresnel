//! The sampler of each cluster in the Connections view: the state of each list with its
//! reason and the time of its last answer, the next poll and the size of the history.

use fresnel_core::cluster::ClusterId;
use fresnel_core::metrics::{Metrics, PollState};

use super::{failed, of_pods, scene_metrics};
use crate::fake::{SECOND, absent, connections_window, error, now, view};

#[test]
fn each_cluster_shows_the_state_of_each_list_of_its_sampler_with_its_reason() {
    let unavailable = PollState::Unavailable(error("service unavailable (HTTP 503)"));
    let mut never = Metrics::waiting(ClusterId::new("lon"), 15 * SECOND, now() - 60 * SECOND);
    never.nodes.state = PollState::Forbidden(error("nodes.metrics.k8s.io is forbidden"));
    let clusters = vec![
        of_pods("ams", scene_metrics("ams", 1)),
        of_pods("fra", failed(scene_metrics("fra", 1), unavailable, now() - 40 * SECOND)),
        of_pods("lon", never),
    ];
    let window = connections_window(view(clusters));
    let texts = [
        "ok, last answer 5 s ago",
        "metrics-server not available: service unavailable (HTTP 503) since 07:59:20; last answer 5 s ago",
        "waits for its first answer",
        "forbidden: nodes.metrics.k8s.io is forbidden since 07:59:00; no answer yet",
        "A poll each 15 s, next in 10 s. History: 30 series, 116Ki.",
        "A poll each 15 s. History: 0 series, 0.",
    ];
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
}
