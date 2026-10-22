//! The time of a frame of the overviews with 10,000 pods in each of four clusters (plan
//! sections 7.3 and 15): the worker thread counts, so a frame reads the counts only, also
//! in a stream of snapshots. The limit of 16 ms holds in a release build, as in
//! `frame_time.rs` of the Pods view, whose steps this test uses.

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ClusterRow;
use fresnel_core::store::StoreState;
use k8s_openapi::api::core::v1::Pod;

use super::cluster;
use crate::data::{held, many_pods, store};
use crate::fake::{Window, view, window_with_no_gpu};
use crate::frame_time::Step;
use crate::timing::timed;

const CLUSTERS: [&str; 4] = ["ams-prod", "fra-prod", "nyc-prod", "sgp-prod"];

/// The clusters of the overview, each with these pods.
fn clusters(pods: &[Arc<Pod>]) -> Vec<ClusterRow> {
    let with_pods = |name: &str| {
        let row = cluster(name, 1, "v1.33.5+k3s1");
        let mut data = row.data.clone().expect("data");
        data.pods = held(store(StoreState::Live, pods));
        ClusterRow { data: Some(data), ..row }
    };
    CLUSTERS.iter().map(|name| with_pods(name)).collect()
}

/// True when each block has the count of these pods.
fn counted(window: &Window, pods: &str) -> bool {
    window.query_all_by_label(pods).count() == CLUSTERS.len() && window.query_by_label("counting…").is_none()
}

#[test]
fn a_frame_of_an_overview_of_four_clusters_with_10_000_pods_each_stays_in_the_frame_limit() {
    timed(|pass| {
        let mut all = many_pods(10_000);
        let mut window = window_with_no_gpu(view(clusters(&all)), 2.0);
        let mut first = Step::new("overview, the first counts");
        let entry = window.get_by_role_and_label(Role::RadioButton, "Cluster overview").rect().center();
        first.click(&mut window, entry);
        first.until(&mut window, "the counts of each cluster", |window| counted(window, "10,000 pods"));
        first.check(pass);

        // A stream of snapshots: 20 pods of each cluster change before each frame.
        let mut stream = Step::new("overview in a stream of snapshots");
        for frame in 0..120_usize {
            for change in 0..20 {
                let index = (frame * 20 + change) * 7 % all.len();
                let mut pod = Pod::clone(&all[index]);
                pod.metadata.resource_version = Some(format!("{frame}"));
                all[index] = Arc::new(pod);
            }
            window.state_mut().view = view(clusters(&all));
            stream.frame(&mut window);
        }
        stream.until(&mut window, "the counts of the last snapshot", |window| counted(window, "10,000 pods"));
        stream.check(pass);

        let mut workloads = Step::new("overview of the workloads");
        let entry = window.get_by_role_and_label(Role::RadioButton, "Workloads overview").rect().center();
        workloads.click(&mut window, entry);
        let total = |window: &Window| window.query_all_by_label_contains(": 10,000 objects").count() == CLUSTERS.len();
        workloads.until(&mut window, "the pods of each cluster", total);
        workloads.check(pass);
    });
}
