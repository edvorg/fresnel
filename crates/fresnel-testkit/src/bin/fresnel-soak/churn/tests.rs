use std::collections::BTreeSet;

use super::*;

/// The objects of the namespace after the changes, as the server would have them.
#[derive(Default)]
struct Namespace {
    config_maps: BTreeSet<String>,
    pods: BTreeSet<String>,
    deployments: BTreeSet<String>,
}

impl Namespace {
    /// Applies one change. Gives a problem for a change that would fail on the server: a
    /// create of an object that exists, or a change of one that does not.
    fn apply(&mut self, change: &[String]) -> Result<(), String> {
        let words: Vec<&str> = change.iter().map(String::as_str).collect();
        let (made, gone) = match words.as_slice() {
            ["create", "configmap", name, ..] => (self.config_maps.insert((*name).to_owned()), true),
            ["patch", "configmap", name, ..] => (true, self.config_maps.contains(*name)),
            ["delete", "configmap", name, ..] => (true, self.config_maps.remove(*name)),
            ["run", name, ..] => (self.pods.insert((*name).to_owned()), true),
            ["delete", "pod", name, ..] => (true, self.pods.remove(*name)),
            ["create", "deployment", name, ..] => (self.deployments.insert((*name).to_owned()), true),
            ["delete", "deployment", name, ..] => (true, self.deployments.remove(*name)),
            ["scale", "deployment", ..] | ["rollout", "restart", ..] => (true, true),
            _ => return Err(format!("an unknown change: {words:?}")),
        };
        if made && gone { Ok(()) } else { Err(format!("a change that fails: {words:?}")) }
    }
}

/// A change that fails on the server counts as failed and changes nothing: each change must
/// find its object, and the objects of the soak stay below their limits.
#[test]
fn each_change_finds_its_object_and_the_objects_stay_below_their_limits() {
    let mut changes = Changes::new(fastrand::Rng::with_seed(7));
    let mut namespace = Namespace::default();
    let mut kinds = BTreeSet::new();
    for _ in 0..10_000 {
        let change = changes.next();
        namespace.apply(&change).unwrap_or_else(|problem| panic!("{problem}"));
        // The second word of `run` is the name of its pod.
        let kind = change.get(1).filter(|_| change.first().is_some_and(|first| first != "run"));
        kinds.insert(format!("{} {}", change.first().cloned().unwrap_or_default(), kind.map_or("pod", String::as_str)));
        assert!(namespace.config_maps.len() as u64 <= CONFIG_MAPS);
        assert!(namespace.pods.len() <= PODS && namespace.deployments.len() <= DEPLOYMENTS);
    }
    assert_eq!(kinds.len(), 9, "each kind of change comes: {kinds:?}");
}

/// A soak runs again with its seed: the same seed gives the same changes.
#[test]
fn the_same_seed_gives_the_same_changes() {
    let changes = |seed| {
        let mut changes = Changes::new(fastrand::Rng::with_seed(seed));
        (0..200).map(|_| changes.next()).collect::<Vec<_>>()
    };
    assert_eq!(changes(3), changes(3));
    assert_ne!(changes(3), changes(4));
}
