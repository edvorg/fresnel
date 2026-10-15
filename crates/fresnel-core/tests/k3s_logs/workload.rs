//! The log of a workload (plan section 12.7): the view of a Deployment has the lines of each
//! pod. A container that starts again gets a marker and its new lines, and a pod that the
//! Deployment replaces ends while its new pod gets a source.

use std::collections::BTreeMap;

use fresnel_core::kinds::{Kind, PodStatus};
use fresnel_core::logs::{LogOptions, LogView, SourceState};

use crate::bed::{BUSYBOX, Bed, LIMIT, NAMESPACE, lines, sources};

/// Each pod tells its start, counts until the file `/tmp/stop` exists, and then ends with
/// an error, so that its container starts again.
const WEB: &str = "echo \"$HOSTNAME up\"; i=0; while [ ! -f /tmp/stop ]; do echo \"$HOSTNAME $i\"; i=$((i+1)); \
                   sleep 0.1; done; echo \"$HOSTNAME stop\"; exit 1";

/// The lines of each pod of the view, with the pod name cut from each line.
fn by_pod(view: &LogView) -> BTreeMap<String, Vec<String>> {
    let mut pods: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (pod, text) in lines(view) {
        let text = text.strip_prefix(&format!("{pod} ")).map(str::to_owned).unwrap_or(text);
        pods.entry(pod).or_default().push(text);
    }
    pods
}

/// True when the lines of one run are "up", then 0, 1, 2 and more in sequence, and maybe
/// "stop" at the end.
fn one_run(lines: &[String]) -> bool {
    let Some((first, rest)) = lines.split_first() else {
        return false;
    };
    let rest = rest.strip_suffix(&["stop".to_owned()]).unwrap_or(rest);
    first == "up" && rest.iter().enumerate().all(|(index, text)| *text == index.to_string())
}

pub(crate) async fn a_container_that_starts_again_and_a_new_pod_stay_in_the_view_of_the_deployment(bed: &Bed) {
    bed.kubectl(&["create", "deployment", "web", "--image", BUSYBOX, "--replicas", "2", "--", "sh", "-c", WEB]).await;
    let timeout = format!("--timeout={}s", LIMIT.as_secs());
    bed.kubectl(&["rollout", "status", "deployment/web", "--namespace", NAMESPACE, &timeout]).await;
    let uid = bed.uid("deployment", "web").await;
    let mut viewer = bed.view(Kind::Deployment, "web", &uid, LogOptions { tail: None, ..LogOptions::default() });
    let view = viewer
        .until("two pods with lines", |view| {
            view.sources.len() == 2 && view.sources.iter().all(|source| source.lines >= 5)
        })
        .await;
    let (first, second) = (view.sources[0].pod.name.to_string(), view.sources[1].pod.name.to_string());

    // The container of the first pod ends and starts again.
    bed.kubectl(&["exec", &first, "--namespace", NAMESPACE, "--", "touch", "/tmp/stop"]).await;
    let marker = format!("marker: the container {} started again", view.sources[0].container);
    let view = viewer
        .until("the second run of the first pod", |view| {
            let lines = by_pod(view).remove(&first).unwrap_or_default();
            lines.iter().position(|text| *text == marker).is_some_and(|at| lines.len() >= at + 4)
        })
        .await;
    let lines = by_pod(&view).remove(&first).unwrap_or_default();
    let at = lines.iter().position(|text| *text == marker).unwrap_or_default();
    let (before, after) = (&lines[..at], &lines[at + 1..]);
    assert!(one_run(before) && before.last().map(String::as_str) == Some("stop"), "the first run: {before:?}");
    assert!(one_run(after), "the second run: {after:?}");

    // The Deployment replaces the second pod.
    bed.kubectl(&["delete", "pod", &second, "--namespace", NAMESPACE, "--grace-period=1", "--wait=false"]).await;
    let view = viewer
        .until("a new pod with lines and the end of the old one", |view| {
            let old = view.sources.iter().find(|source| *source.pod.name == *second);
            let new = view.sources.iter().find(|source| *source.pod.name != *first && *source.pod.name != *second);
            old.is_some_and(|source| source.state.is_final()) && new.is_some_and(|source| source.lines >= 3)
        })
        .await;
    let new = view.sources.iter().find(|source| *source.pod.name != *first && *source.pod.name != *second);
    let new = new.map(|source| source.pod.name.to_string()).unwrap_or_default();
    bed.until_pod(&new, "runs", |pod| PodStatus::of(pod).text == "Running").await;
    let pods = by_pod(&view);
    assert!(one_run(&pods[&new]), "the new pod: {:?}", pods[&new]);
    assert!(one_run(&pods[&second]), "the old pod keeps its lines: {:?}", pods[&second]);
    let open = view.sources.iter().filter(|source| source.state == SourceState::Streaming).count();
    assert_eq!(open, 2, "{:#?}", sources(&view));
}
