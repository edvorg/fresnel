//! A stream that breaks starts again and loses no line (plan section 15, M9): a pod counts
//! its lines, the proxy resets each connection of the app, and the view has each number one
//! time, in sequence, with one marker at the break.

use fresnel_core::kinds::{Kind, PodStatus};
use fresnel_core::logs::{LogOptions, LogView, SourceState};

use crate::bed::{BUSYBOX, Bed, lines};
use crate::common::in_time;

/// A line each 50 ms with its number.
const COUNTER: &str = "i=0; while true; do echo \"line $i\"; i=$((i+1)); sleep 0.05; done";
const MARKER: &str = "marker: the stream started again after: ";

/// The numbers of the log lines of the view, and the count of its markers.
fn numbers(view: &LogView) -> (Vec<u64>, usize) {
    let lines = lines(view);
    let markers = lines.iter().filter(|(_, text)| text.starts_with("marker: ")).count();
    let number = |(_, text): &(String, String)| text.strip_prefix("line ")?.parse().ok();
    (lines.iter().filter_map(number).collect(), markers)
}

pub(crate) async fn a_break_of_each_connection_loses_no_line_and_shows_one_marker(bed: &Bed) {
    bed.kubectl(&["run", "counter", "--image", BUSYBOX, "--restart", "Always", "--command", "--", "sh", "-c", COUNTER])
        .await;
    bed.until_pod("counter", "runs", |pod| PodStatus::of(pod).text == "Running").await;
    let uid = bed.uid("pod", "counter").await;
    let options = LogOptions { tail: None, ..LogOptions::default() };
    let mut viewer = bed.view(Kind::Pod, "counter", &uid, options);
    viewer.until("40 lines", |view| view.lines.len() >= 40).await;

    let resets = bed.proxy.counters().reset;
    in_time("the reset of each connection", bed.proxy.reset_all()).await;
    assert!(bed.proxy.counters().reset > resets, "the proxy reset the connection of the stream");
    let view = viewer
        .until("40 lines after the marker of the break", |view| {
            let lines = lines(view);
            let after = lines.iter().position(|(_, text)| text.starts_with(MARKER));
            after.is_some_and(|marker| lines.len() >= marker + 41) && view.sources[0].state == SourceState::Streaming
        })
        .await;
    let (numbers, markers) = numbers(&view);
    let expected: Vec<u64> = (0..numbers.len() as u64).collect();
    assert!(numbers == expected, "each line one time and no gap: {numbers:?}");
    assert_eq!(markers, 1, "one marker for the break: {:?}", lines(&view));
    assert!(view.sources[0].repeated >= 1, "the new stream sent the lines of the latest second again");
    eprintln!("restart: {} lines, {} sent two times", numbers.len(), view.sources[0].repeated);
}
