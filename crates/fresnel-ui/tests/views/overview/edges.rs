//! The overviews at their edges: a stream of snapshots.

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ClusterRow;
use fresnel_core::store::StoreState;

use super::{cluster, open};
use crate::data::{held, pods, store};
use crate::fake::{shows, until, view, window};

/// The cluster of the scene with its first `count` pods.
fn with_pods(count: usize) -> ClusterRow {
    let row = cluster("ams-prod", 1, "v1.33.5+k3s1");
    let mut data = row.data.clone().expect("data");
    data.pods = held(store(StoreState::Live, pods().get(..count).expect("the pods")));
    ClusterRow { data: Some(data), ..row }
}

/// A busy cluster gives new snapshots each few frames, and each one asks for new counts
/// that come some milliseconds later. The page tells that it counts only after a wait, as
/// the table does: a note in each frame after a change would flicker.
#[test]
fn a_stream_of_snapshots_gives_new_counts_with_no_note_in_the_frame_of_each_change() {
    let mut window = window(view(vec![with_pods(25)]));
    open(&mut window, "Cluster overview");
    let mut noted = 0;
    for round in 0..20 {
        let count = 24 + round % 2;
        window.state_mut().view = view(vec![with_pods(count)]);
        window.step();
        noted += usize::from(window.query_by_label("counting…").is_some());
        until(&mut window, "the new counts", |window| shows(window, &format!("{count} pods")));
    }
    assert_eq!(noted, 0, "frames of a change with the note");
}
