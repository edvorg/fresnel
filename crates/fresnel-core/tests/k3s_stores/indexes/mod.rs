//! A scene on k3s with one object for each relation and each state that the indexes, the
//! Pod status text and the health values read.
//!
//! - `scene`: the objects of the scene, and what kubectl shows of its pods.
//! - `fixture`: the copy of the scene for the unit tests.

mod fixture;
mod scene;

use crate::bed::Cluster;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn the_scene_comes_to_rest_with_each_pod_in_its_state() {
    let cluster = Cluster::start("indexes").await;
    let took = scene::create(&cluster).await;
    println!("the scene is at rest after {took:?}:\n{}", scene::pod_lines(&cluster).await.0);
    fixture::write_if_asked(&cluster).await;
}
