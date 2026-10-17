//! The first objects of the soak: a namespace, a Deployment whose pod writes numbered lines
//! for the log view, a Deployment that the churn scales and restarts, and two pods behind a
//! Service that serve a page for the forward of the soak.

use fresnel_testkit::K3s;

/// The namespace of each object of the soak.
pub(crate) const NAMESPACE: &str = "soak";
/// The Deployment of the log view, as `fresnel-cli soak --log` takes it.
pub(crate) const LOGGER: &str = "soak/soak-logger";
/// The Deployment that the churn scales and restarts.
pub(crate) const WEB: &str = "soak-web";
/// The Service of the forward, as `fresnel-cli soak --forward` takes it, and its Deployment,
/// which the churn restarts.
pub(crate) const FORWARD: &str = "soak/soak-echo:80";
pub(crate) const ECHO: &str = "soak-echo";
/// The page of the pods of the forward: the check of `fresnel-cli soak` looks for its text.
const PAGE: &str = "mkdir -p /www && echo soak-echo > /www/index.html && exec httpd -f -p 8080 -h /www";
/// The image of the logger. The log tests of the core use it too.
const BUSYBOX: &str = "rancher/mirrored-library-busybox:1.36.1";
/// The image of each other pod: it does nothing and starts fast.
pub(crate) const PAUSE: &str = "rancher/mirrored-pause:3.6";

/// The first objects. The logger writes lines with a number, "soak 0", "soak 1" and so on,
/// with `pause` seconds between two lines. A recreate of the cluster creates them again.
pub(crate) async fn create(k3s: &K3s, pause: &str) -> Result<(), String> {
    let lines = format!("i=0; while true; do echo \"soak $i\"; i=$((i+1)); sleep {pause}; done");
    let echo = ["--namespace", NAMESPACE, "create", "deployment", ECHO, "--image", BUSYBOX, "--replicas", "2"];
    let echo = [&echo[..], &["--port", "8080", "--", "sh", "-c", PAGE]].concat();
    let steps: [&[&str]; 8] = [
        &["create", "namespace", NAMESPACE],
        &[
            "--namespace",
            NAMESPACE,
            "create",
            "deployment",
            "soak-logger",
            "--image",
            BUSYBOX,
            "--",
            "sh",
            "-c",
            &lines,
        ],
        &["--namespace", NAMESPACE, "create", "deployment", WEB, "--image", PAUSE, "--replicas", "2"],
        &echo,
        &["--namespace", NAMESPACE, "expose", "deployment", ECHO, "--port", "80", "--target-port", "8080"],
        &["--namespace", NAMESPACE, "rollout", "status", "deployment/soak-logger", "--timeout=180s"],
        &["--namespace", NAMESPACE, "rollout", "status", "deployment/soak-web", "--timeout=180s"],
        &["--namespace", NAMESPACE, "rollout", "status", "deployment/soak-echo", "--timeout=180s"],
    ];
    for step in steps {
        k3s.kubectl(step).await.map_err(|error| format!("the scene: kubectl {}: {error}", step.join(" ")))?;
    }
    Ok(())
}
