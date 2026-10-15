//! PNG files of the overviews in the dark theme, for a look by a person: the Cluster
//! overview of two clusters, one of them not live, and the Workloads overview.

use super::cluster::degraded;
use super::{cluster, open};
use crate::fake::{view, window_with};
use crate::snapshots::{SCALE, save_clean};

#[test]
fn the_cluster_and_the_workloads_overviews() {
    let clusters = vec![cluster("ams-prod", 1, "v1.33.5+k3s1"), degraded("fra-prod")];
    let mut window = window_with(view(clusters.clone()), egui::Theme::Dark, SCALE);
    window.set_size(egui::vec2(crate::fake::WINDOW.x, 1100.0));
    open(&mut window, "Cluster overview");
    save_clean(&mut window, "m7-overview-cluster");
    let mut window = window_with(view(clusters), egui::Theme::Dark, SCALE);
    open(&mut window, "Workloads overview");
    save_clean(&mut window, "m7-overview-workloads");
}
