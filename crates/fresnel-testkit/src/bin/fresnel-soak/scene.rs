//! The first objects of the soak: a namespace, a Deployment whose pod writes numbered lines
//! for the log view, and a Deployment that the churn scales and restarts.

use fresnel_testkit::K3s;

/// The namespace of each object of the soak.
pub(crate) const NAMESPACE: &str = "soak";
/// The Deployment of the log view, as `fresnel-cli soak --log` takes it.
pub(crate) const LOGGER: &str = "soak/soak-logger";
/// The Deployment that the churn scales and restarts.
pub(crate) const WEB: &str = "soak-web";
/// The image of the logger. The log tests of the core use it too.
const BUSYBOX: &str = "rancher/mirrored-library-busybox:1.36.1";
/// The image of each other pod: it does nothing and starts fast.
pub(crate) const PAUSE: &str = "rancher/mirrored-pause:3.6";
/// Two lines each second, each with its number: "soak 0", "soak 1" and so on.
const LINES: &str = "i=0; while true; do echo \"soak $i\"; i=$((i+1)); sleep 0.5; done";

pub(crate) async fn create(k3s: &K3s) -> Result<(), String> {
    let steps: [&[&str]; 5] = [
        &["create", "namespace", NAMESPACE],
        &["--namespace", NAMESPACE, "create", "deployment", "soak-logger", "--image", BUSYBOX, "--", "sh", "-c", LINES],
        &["--namespace", NAMESPACE, "create", "deployment", WEB, "--image", PAUSE, "--replicas", "2"],
        &["--namespace", NAMESPACE, "rollout", "status", "deployment/soak-logger", "--timeout=180s"],
        &["--namespace", NAMESPACE, "rollout", "status", "deployment/soak-web", "--timeout=180s"],
    ];
    for step in steps {
        k3s.kubectl(step).await.map_err(|error| format!("the scene: kubectl {}: {error}", step.join(" ")))?;
    }
    Ok(())
}
