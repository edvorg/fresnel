//! The changes of the soak: ConfigMaps, pods and Deployments that come, change and go, one
//! change each step of the pace, so each store of the app has new events at each check. A restart of the
//! Deployment of the forward moves its tunnels to new pods. A change that fails, for example
//! during a restart of the API server, counts and the next one follows.

use std::collections::{BTreeSet, VecDeque};
use std::time::Duration;

use fresnel_testkit::K3s;
use tokio::time::sleep;

use super::Never;
use super::scene::{ECHO, NAMESPACE, PAUSE, WEB};

/// The most ConfigMaps, bare pods and extra Deployments at one time.
const CONFIG_MAPS: u64 = 40;
const PODS: usize = 10;
const DEPLOYMENTS: usize = 3;

pub(crate) struct Churn<'a> {
    k3s: &'a K3s,
    step: Duration,
    changes: Changes,
    done: u64,
    failed: u64,
    first_failure: Option<String>,
}

impl<'a> Churn<'a> {
    pub(crate) fn new(k3s: &'a K3s, rng: fastrand::Rng, step: Duration) -> Self {
        Self { k3s, step, changes: Changes::new(rng), done: 0, failed: 0, first_failure: None }
    }

    pub(crate) async fn run(&mut self) -> Never {
        loop {
            sleep(self.step).await;
            let change = self.changes.next();
            let arguments: Vec<&str> = change.iter().map(String::as_str).collect();
            match self.k3s.kubectl(&[&["--namespace", NAMESPACE], &arguments[..]].concat()).await {
                Ok(_) => self.done += 1,
                Err(error) => {
                    self.failed += 1;
                    self.first_failure.get_or_insert_with(|| format!("kubectl {}: {error}", arguments.join(" ")));
                }
            }
        }
    }

    pub(crate) fn summary(&self) -> String {
        let failure = self.first_failure.as_deref().map_or_else(String::new, |first| format!(" (first: {first})"));
        format!("{} changes, {} failed{failure}", self.done, self.failed)
    }
}

/// The objects that the changes made, so that each change finds its object, and the count of
/// the changes, which names the new objects.
struct Changes {
    rng: fastrand::Rng,
    count: u64,
    /// The numbers of the ConfigMaps that exist.
    config_maps: BTreeSet<u64>,
    pods: VecDeque<String>,
    deployments: VecDeque<String>,
}

impl Changes {
    fn new(rng: fastrand::Rng) -> Self {
        Self { rng, count: 0, config_maps: BTreeSet::new(), pods: VecDeque::new(), deployments: VecDeque::new() }
    }

    /// The arguments of the next change: mostly ConfigMaps, then pods, then the Deployments.
    fn next(&mut self) -> Vec<String> {
        self.count += 1;
        let words = |words: &[&str]| words.iter().map(|word| (*word).to_owned()).collect::<Vec<_>>();
        let count = self.count.to_string();
        let number = self.rng.u64(0..CONFIG_MAPS);
        let name = format!("cm-{number:02}");
        match self.rng.u32(0..12) {
            0..=5 if !self.config_maps.contains(&number) => {
                self.config_maps.insert(number);
                words(&["create", "configmap", &name, &format!("--from-literal=round={count}")])
            }
            0..=4 => {
                let patch = format!("{{\"data\":{{\"round\":\"{count}\"}}}}");
                words(&["patch", "configmap", &name, "--type", "merge", "--patch", &patch])
            }
            5 => {
                self.config_maps.remove(&number);
                words(&["delete", "configmap", &name, "--wait=false"])
            }
            6..=7 if self.pods.len() >= PODS => {
                let name = self.pods.pop_front().unwrap_or_default();
                words(&["delete", "pod", &name, "--wait=false"])
            }
            6..=7 => {
                let name = format!("pod-{count}");
                self.pods.push_back(name.clone());
                words(&["run", &name, "--image", PAUSE, "--restart", "Never"])
            }
            8..=9 => words(&["scale", "deployment", WEB, &format!("--replicas={}", self.rng.u32(1..=4))]),
            10 if self.count.is_multiple_of(2) => words(&["rollout", "restart", &format!("deployment/{ECHO}")]),
            10 => words(&["rollout", "restart", &format!("deployment/{WEB}")]),
            _ if self.deployments.len() >= DEPLOYMENTS => {
                let name = self.deployments.pop_front().unwrap_or_default();
                words(&["delete", "deployment", &name, "--wait=false"])
            }
            _ => {
                let name = format!("app-{count}");
                self.deployments.push_back(name.clone());
                words(&["create", "deployment", &name, "--image", PAUSE])
            }
        }
    }
}

#[cfg(test)]
mod tests;
