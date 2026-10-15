//! 1,000,000 lines keep the memory of a view bounded (plan section 15, M9): eight pods of a
//! Deployment write 125,000 lines each, one view reads all of them, and the bytes that the
//! process holds stay near the limit of its ring.
//!
//! The kubelet rotates a container log above 10 MiB, and a follow can stop at a rotation.
//! The log of each pod has some 6 MB, so no log rotates.

use fresnel_core::kinds::Kind;
use fresnel_core::logs::{LogOptions, LogSettings};

use crate::bed::{BUSYBOX, Bed, LIMIT, NAMESPACE, sources};
use crate::memory;

const PODS: u64 = 8;
const LINES: u64 = 125_000;
/// The most bytes that the view can add to the process: its ring of 200,000 short lines
/// needs some 16 MB, and the streams, the merge and the published copies need some more.
/// A view that kept each line would need more than 80 MB.
const BOUND: usize = 64 * 1024 * 1024;

pub(crate) async fn one_million_lines_keep_the_memory_of_a_view_bounded(bed: &Bed) {
    let script = format!("seq 1 {LINES}; sleep 3600");
    let replicas = PODS.to_string();
    let create =
        ["create", "deployment", "million", "--image", BUSYBOX, "--replicas", &replicas, "--", "sh", "-c", &script];
    bed.kubectl(&create).await;
    let timeout = format!("--timeout={}s", LIMIT.as_secs());
    bed.kubectl(&["rollout", "status", "deployment/million", "--namespace", NAMESPACE, &timeout]).await;
    let uid = bed.uid("deployment", "million").await;

    let before = memory::live();
    let mut highest = before;
    let mut viewer = bed.view(Kind::Deployment, "million", &uid, LogOptions { tail: None, ..LogOptions::default() });
    let limits = LogSettings::default().ring;
    let view = viewer
        .until("each line of each pod", |view| {
            highest = highest.max(memory::live());
            assert!(view.lines.len() <= limits.lines && view.bytes <= limits.bytes, "the ring is inside its limits");
            let lines: u64 = view.sources.iter().map(|source| source.lines).sum();
            view.sources.len() as u64 == PODS && lines == PODS * LINES && view.lines.end() == PODS * LINES
        })
        .await;
    assert_eq!((view.lines.len(), view.dropped), (200_000, 800_000), "{:#?}", sources(&view));
    let added = highest.saturating_sub(before);
    eprintln!("million: the view added {} MB at most, the ring has {} MB", added / 1_000_000, view.bytes / 1_000_000);
    assert!(added < BOUND, "the view added {added} bytes");
    drop(view);
    drop(viewer);
}
