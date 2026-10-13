//! The indexes, the Pod status text and the health values on a real cluster: a scene on
//! k3s with one object for each relation and each state that they read.
//!
//! - `scene`: the objects of the scene, and what kubectl shows of its pods.
//! - `fixture`: the copy of the scene for the unit tests.
//! - `read`: the API and kubectl as the test reads them. `status`: the Pod status text and
//!   the health values. `owners`: the owner index. `others`: the two other indexes.

mod fixture;
mod others;
mod owners;
mod read;
mod scene;
mod status;

use fresnel_core::store::StoreSettings;

use crate::api::assert_equal_to_the_api;
use crate::bed::{App, Cluster, LIMIT};

/// The steps are in one test, because the start of a cluster is the slow part.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn the_indexes_and_the_pod_status_agree_with_the_api_and_with_kubectl() {
    let cluster = Cluster::start("indexes").await;
    let took = scene::create(&cluster).await;
    println!("the scene is at rest after {took:?}:\n{}", scene::pod_lines(&cluster).await.0);
    fixture::write_if_asked(&cluster).await;

    let app = App::start(&cluster, StoreSettings::default()).await;
    let indexes = app.stores.indexes(&cluster.tasks);
    app.stores.until_live(LIMIT).await;
    assert_equal_to_the_api(&app.stores.stores, &cluster.api.gateway, LIMIT).await;

    status::each_pod_has_the_row_of_kubectl_and_of_the_api(&cluster, &app).await;
    status::the_objects_of_the_scene_have_their_health_value(&app);
    owners::each_owner_has_the_pods_of_the_api_and_of_kubectl(&cluster, &indexes).await;
    owners::each_service_has_the_pods_of_kubectl_and_of_the_api(&cluster, &indexes).await;
    others::each_object_has_the_pods_that_use_it(&cluster, &app, &indexes).await;
    others::each_object_has_its_events(&cluster, &app, &indexes).await;
    others::a_new_pod_of_a_deployment_reaches_the_index(&cluster, &indexes).await;
    drop(indexes);
    assert_eq!(app.proxy.task_failures(), Vec::<String>::new());
}
