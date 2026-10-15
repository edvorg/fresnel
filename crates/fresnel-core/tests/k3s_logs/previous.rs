//! The log of the instance of a container before its crash (plan section 12.7): the view
//! has the lines of `kubectl logs --previous` and ends, and the view of the current
//! instance has the lines of the run after the crash.
//!
//! The container crashes one time and then runs: in a loop of crashes, the kubelet removes
//! the instance before the last one at once, and for some seconds the status still names
//! it (seen on k3s 1.33). kubectl then gets no earlier log, and the test would compare
//! nothing. The source of the app waits through that time; its unit test covers it.

use fresnel_core::kinds::Kind;
use fresnel_core::logs::{LogOptions, LogView, SourceState};
use k8s_openapi::api::core::v1::Pod;

use crate::bed::{BUSYBOX, Bed, NAMESPACE, lines};

/// The first run crashes, and each later run finds the mark of the first one in the volume
/// of the pod.
const CRASH: &str = "if [ -f /mark/ran ]; then echo \"the next run\"; exec sleep 3600; fi; touch /mark/ran; \
                     echo \"first of the run\"; echo \"second of the run\"; exit 3";

/// The pod with a volume for the mark.
fn overrides() -> String {
    let container = serde_json::json!({"name": "crash", "image": BUSYBOX, "command": ["sh", "-c", CRASH],
        "volumeMounts": [{"name": "mark", "mountPath": "/mark"}]});
    serde_json::json!({"spec": {"volumes": [{"name": "mark", "emptyDir": {}}], "containers": [container]}}).to_string()
}

/// True when the container runs after a crash that has an ID.
fn crashed_once(pod: &Pod) -> bool {
    let status =
        pod.status.as_ref().and_then(|status| status.container_statuses.as_ref()).and_then(|list| list.first());
    let runs = status.and_then(|status| status.state.as_ref()).is_some_and(|state| state.running.is_some());
    let last = status.and_then(|status| status.last_state.as_ref()).and_then(|state| state.terminated.as_ref());
    runs && last.is_some_and(|run| run.container_id.is_some())
}

/// The texts of the lines of a view.
fn texts(view: &LogView) -> Vec<String> {
    lines(view).into_iter().map(|(_, text)| text).collect()
}

pub(crate) async fn the_earlier_instance_of_a_crashed_container_has_its_lines(bed: &Bed) {
    let overrides = overrides();
    let run = ["run", "crash", "--image", BUSYBOX, "--restart", "Always", "--overrides", &overrides];
    bed.kubectl(&run).await;
    bed.until_pod("crash", "runs after a crash", crashed_once).await;
    let uid = bed.uid("pod", "crash").await;
    let kubectl = bed.kubectl(&["logs", "crash", "--previous", "--namespace", NAMESPACE]).await;
    assert_eq!(kubectl.lines().collect::<Vec<_>>(), ["first of the run", "second of the run"]);

    let options = LogOptions { previous: true, tail: None, ..LogOptions::default() };
    let mut viewer = bed.view(Kind::Pod, "crash", &uid, options);
    let ended = SourceState::Ended { reason: "the view has the full log of the earlier instance".into() };
    let view = viewer
        .until("the full log of the earlier instance", |view| {
            view.sources.first().is_some_and(|source| source.state == ended)
        })
        .await;
    assert_eq!(texts(&view), ["first of the run", "second of the run"], "the lines of kubectl logs --previous");
    assert_eq!(view.sources[0].container.as_ref(), "crash");

    let mut current = bed.view(Kind::Pod, "crash", &uid, LogOptions { tail: None, ..LogOptions::default() });
    let view = current.until("the line of the run after the crash", |view| !view.lines.is_empty()).await;
    assert_eq!(
        (texts(&view), view.sources[0].state.clone()),
        (vec!["the next run".to_owned()], SourceState::Streaming)
    );
}
