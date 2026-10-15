//! The objects of the test: the same ones in the namespace `app` of each cluster, made by
//! kubectl. The pods use the pause image of k3s, which each node has.

use fresnel_core::app::CoreView;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::K3s;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::batch::v1::CronJob;
use k8s_openapi::api::core::v1::Secret;
use tokio::time::{Instant, sleep};

use crate::bed::{LIMIT, snapshot};

/// The namespace of the objects.
pub(crate) const APP: &str = "app";
const IMAGE: &str = "rancher/mirrored-pause:3.6";
/// The Deployments: `web` has two revisions, `old` goes in the test, and the image of
/// `stuck` does not exist, so its rollout never completes.
const DEPLOYMENTS: [(&str, &str); 3] = [("web", IMAGE), ("old", IMAGE), ("stuck", "rancher/not-there:0")];

/// Runs kubectl in the cluster as the cluster admin.
pub(crate) async fn kubectl(k3s: &K3s, arguments: &[&str]) -> String {
    k3s.kubectl(arguments).await.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
}

/// One field of an object of the namespace, as kubectl reads it with a JSONPath.
pub(crate) async fn field(k3s: &K3s, object: &str, path: &str) -> String {
    let jsonpath = format!("jsonpath={path}");
    kubectl(k3s, &["get", object, "--namespace", APP, "--ignore-not-found", "--output", &jsonpath]).await
}

pub(crate) async fn create(k3s: &K3s) {
    kubectl(k3s, &["create", "namespace", APP]).await;
    for (name, image) in DEPLOYMENTS {
        kubectl(k3s, &["create", "deployment", name, "--image", image, "--namespace", APP]).await;
    }
    // The second revision starts after the first one exists: a change before the controller
    // read the first template would give one revision.
    until_revisions(k3s, 1).await;
    kubectl(k3s, &["set", "env", "deployment/web", "X=2", "--namespace", APP]).await;
    until_revisions(k3s, 2).await;
    let schedule = "0 0 1 1 *";
    kubectl(k3s, &["create", "cronjob", "nightly", "--image", IMAGE, "--schedule", schedule, "--namespace", APP]).await;
    let secret = ["create", "secret", "generic", "creds", "--namespace", APP];
    kubectl(k3s, &[&secret[..], &["--from-literal=user=admin", "--from-literal=token=t-1"]].concat()).await;
}

/// Waits until kubectl lists this count of revisions of `web`. The cluster gives no event
/// to the test, so the wait reads again after a short pause.
async fn until_revisions(k3s: &K3s, count: usize) {
    let end = Instant::now() + LIMIT;
    loop {
        let text = kubectl(k3s, &["rollout", "history", "deployment/web", "--namespace", APP]).await;
        let numbers = text.lines().filter(|line| line.split_whitespace().next().is_some_and(is_number));
        if numbers.count() == count {
            return;
        }
        assert!(Instant::now() < end, "in time: {count} revisions of web in {}", k3s.name());
        sleep(std::time::Duration::from_millis(250)).await;
    }
}

fn is_number(text: &str) -> bool {
    text.parse::<u32>().is_ok()
}

/// True when the stores of the core in the cluster have each object of the test.
pub(crate) fn is_loaded(view: &CoreView, cluster: &str) -> bool {
    let key = |name: &str| ObjectKey::new(APP, name);
    let deployments = snapshot::<Deployment>(view, cluster)
        .is_some_and(|store| DEPLOYMENTS.iter().all(|(name, _)| store.items.get(&key(name)).is_some()));
    let cron_job = snapshot::<CronJob>(view, cluster).is_some_and(|store| store.items.get(&key("nightly")).is_some());
    let secret = snapshot::<Secret>(view, cluster).is_some_and(|store| store.items.get(&key("creds")).is_some());
    deployments && cron_job && secret
}

/// What an order could change in a cluster: the generation of each Deployment and
/// CronJob, which grows with each change of its spec, the names of the Jobs, and the stored
/// keys of the Secret. The text has values of the test, so an assertion does not print it.
pub(crate) async fn state(k3s: &K3s) -> String {
    let each = r#"{range .items[*]}{.kind}/{.metadata.name}={.metadata.generation}{"\n"}{end}"#;
    let objects = field(k3s, "deployments,cronjobs,jobs", each).await;
    let secret = field(k3s, "secret/creds", "{.data}").await;
    format!("{objects}{secret}")
}
