//! What the API and kubectl tell of the cluster, as the test itself reads them: with no
//! store and no index of the app.

use std::fmt::Debug;
use std::time::Duration;

use fresnel_core::cluster::Gateway;
use fresnel_testkit::K3s;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ListMeta;
use kube::core::Request;
use kube::core::params::ListParams;
use serde::Deserialize;
use serde_json::Value;
use tokio::time::{Instant, sleep};

/// The longest time for two sources to tell the same.
const LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_millis(200);

#[derive(Deserialize)]
struct Page {
    metadata: ListMeta,
    items: Option<Vec<Value>>,
}

/// Each object of one collection, as the API gives it. `fields` is a field selector.
pub(crate) async fn objects(api: &Gateway, collection: &str, fields: Option<&str>) -> Vec<Value> {
    let request = Request::new(collection);
    let mut params = ListParams::default().limit(500);
    if let Some(fields) = fields {
        params = params.fields(fields);
    }
    let mut objects = Vec::new();
    loop {
        let page: Page = api.list_page(request.list(&params).expect("a request")).await.expect("a list page");
        objects.extend(page.items.unwrap_or_default());
        match page.metadata.continue_.filter(|token| !token.is_empty()) {
            Some(token) => params = params.continue_token(&token),
            None => return objects,
        }
    }
}

/// A text field of the JSON of an object. Empty when the object does not have it.
pub(crate) fn text<'a>(object: &'a Value, pointer: &str) -> &'a str {
    object.pointer(pointer).and_then(Value::as_str).unwrap_or_default()
}

/// The words of each line of a `kubectl get` with the output `custom-columns`.
pub(crate) async fn columns(k3s: &K3s, what: &str, scope: &[&str], columns: &str) -> Vec<Vec<String>> {
    let output = format!("custom-columns={columns}");
    let arguments = [&["get", what], scope, &["--no-headers", "--output", &output]].concat();
    let lines = k3s.kubectl(&arguments).await.unwrap_or_else(|error| panic!("kubectl get {what}: {error}"));
    lines.lines().map(|line| line.split_whitespace().map(str::to_owned).collect()).collect()
}

/// Reads two sources again until they tell the same, to a time limit. The cluster changes
/// by itself: its Events, and the pods of the scene that start again and again.
pub(crate) async fn until_equal<T: PartialEq + Debug>(what: &str, read: impl AsyncFn() -> (T, T)) -> T {
    let end = Instant::now() + LIMIT;
    loop {
        let (left, right) = read().await;
        if left == right {
            return left;
        }
        assert!(Instant::now() < end, "{what}: the two sources differ after {LIMIT:?}:\n{left:#?}\n{right:#?}");
        sleep(STEP).await;
    }
}
