//! The port forwards of the app (plan section 12.3): one list in the core, not in a connected
//! cluster, so a listener outlives each connect and disconnect of its cluster.
//!
//! A cluster with a running forward stays connected, also when the user removes it from the
//! selection; a forward reads, so a read-only cluster forwards too. Each new connect gives
//! the forwards of the cluster its feed. The saved rules of the settings file are in the list
//! from its first read on, and start with the app when `start_saved` says so.

use std::collections::BTreeSet;
use std::sync::Arc;

use fresnel_net::WallClock;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName};

use super::{Inner, Phase, State, data};
use crate::cluster::{ClusterId, LinkState};
use crate::config::Config;
use crate::config::table::Setting;
use crate::forward::{Feed, Forward, ForwardParts, ForwardRule, ForwardSettings, ForwardState, ForwardView};
use crate::published::{Disposal, Held, Notice, Publisher};

/// One forward of the view.
#[derive(Clone)]
pub struct ForwardRow {
    /// A number that grows with each forward of the core. A command names it.
    pub id: u64,
    /// True when the settings file has the rule.
    pub saved: bool,
    /// The forward at this moment.
    pub view: Held<ForwardView>,
}

impl std::fmt::Debug for ForwardRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let view: &ForwardView = &self.view;
        f.debug_struct("ForwardRow").field("id", &self.id).field("saved", &self.saved).field("view", view).finish()
    }
}

struct Entry {
    id: u64,
    saved: bool,
    /// `None` for a forward that stopped.
    running: Option<Forward>,
    /// The last view of a forward that stopped, and the rule of each one.
    last: Held<ForwardView>,
}

impl Entry {
    fn rule(&self) -> &ForwardRule {
        &self.last.rule
    }

    fn view(&self) -> Held<ForwardView> {
        self.running.as_ref().map_or_else(|| self.last.clone(), Forward::view)
    }
}

/// The forwards of the core, in the sequence of their start.
#[derive(Default)]
pub(super) struct Forwards {
    entries: Vec<Entry>,
    next: u64,
    /// Hears each new view of a forward, for the repaint.
    notice: Notice,
    /// The task that asks for a repaint for each new view. It starts with the first forward.
    watch: Option<TaskHandle>,
}

impl Forwards {
    /// The rows of the view.
    pub(super) fn rows(&self) -> Arc<[ForwardRow]> {
        self.entries.iter().map(|entry| ForwardRow { id: entry.id, saved: entry.saved, view: entry.view() }).collect()
    }

    /// The clusters of the forwards that run: each one stays connected.
    pub(super) fn clusters(&self) -> BTreeSet<ClusterId> {
        let running = self.entries.iter().filter(|entry| entry.running.is_some());
        running.map(|entry| entry.rule().cluster.clone()).collect()
    }
}

impl Inner {
    /// Starts a forward of `rule`, or the forward of an equal rule that stopped. An equal
    /// rule that runs keeps its forward.
    pub(super) fn start_forward(&self, rule: ForwardRule) {
        if let Some(problem) = rule.problem() {
            self.journal.warning(rule.cluster.as_str(), format!("{rule}: no forward: {problem}"));
            return;
        }
        let mut state = self.state();
        let known = state.forwards.entries.iter().find(|entry| entry.rule() == &rule);
        let id = match known {
            Some(entry) if entry.running.is_some() => return,
            Some(entry) => entry.id,
            None => {
                let saved = state.config.forwards.contains(&rule);
                self.add_forward(&mut state, rule, saved)
            }
        };
        self.run_forward(&mut state, id);
    }

    /// Starts the forward with this number again: a new listener, and a new state.
    pub(super) fn restart_forward(&self, id: u64) {
        self.run_forward(&mut self.state(), id);
    }

    /// Stops the forward with this number: its listener closes, and each local connection
    /// gets a reset. It stays in the list.
    pub(super) fn stop_forward(&self, id: u64) {
        let mut state = self.state();
        let Some(entry) = state.forwards.entries.iter_mut().find(|entry| entry.id == id) else { return };
        if let Some(forward) = entry.running.take() {
            entry.last = forward.stop();
            let rule = entry.rule().clone();
            self.journal.info(rule.cluster.as_str(), format!("{rule}: stopped by the user"));
        }
        drop(state);
        self.repaint.request();
    }

    /// Removes a forward from the list: it stops first. A saved rule stays in the list, as
    /// a forward that stopped, until the settings file forgets it.
    pub(super) fn remove_forward(&self, id: u64) {
        self.stop_forward(id);
        let mut state = self.state();
        state.forwards.entries.retain(|entry| entry.id != id || entry.saved);
        drop(state);
        self.repaint.request();
    }

    /// "Retry now" of a forward that runs.
    pub(super) fn retry_forward(&self, id: u64) {
        let state = self.state();
        let entry = state.forwards.entries.iter().find(|entry| entry.id == id);
        if let Some(forward) = entry.and_then(|entry| entry.running.as_ref()) {
            forward.retry();
        }
    }

    /// Follows the saved rules of the settings file after a read of it. The first read at
    /// the start of the app starts each saved rule when the settings say so.
    pub(super) fn follow_saved(&self, first: bool) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        let config = Arc::clone(&state.config);
        for entry in &mut state.forwards.entries {
            entry.saved = config.forwards.contains(entry.rule());
        }
        let start = first && config.flag(Setting::StartSaved);
        for rule in &config.forwards {
            if state.forwards.entries.iter().all(|entry| entry.rule() != rule) {
                let id = self.add_forward(&mut state, rule.clone(), true);
                if start {
                    self.run_forward(&mut state, id);
                }
            }
        }
        drop(state);
        self.repaint.request();
    }

    /// Gives each forward that runs the feed of its cluster, or why it has none: after each
    /// change of the connected clusters.
    pub(super) fn feed_forwards(&self) {
        let state = self.state();
        for entry in &state.forwards.entries {
            if let Some(forward) = &entry.running {
                forward.set_feed(self.feed_of(&state, &entry.rule().cluster));
            }
        }
    }

    /// The feed of a connected cluster, or why it has none now.
    fn feed_of(&self, state: &State, cluster: &ClusterId) -> Result<Arc<Feed>, String> {
        if let Some(feed) = state.connected.get(cluster).and_then(|connected| connected.feed()) {
            return Ok(feed);
        }
        let clusters = self.registry.get().map(|registry| registry.snapshot()).unwrap_or_default();
        Err(match clusters.into_iter().find(|view| &view.id == cluster).map(|view| view.state) {
            Some(LinkState::Failed(error)) => format!("the connect of {cluster} failed: {error}"),
            Some(LinkState::Off | LinkState::On(_)) => format!("{cluster} connects"),
            None if state.startup == super::Startup::Ready => format!("no kubeconfig file has the context {cluster}"),
            None => "the app reads the kubeconfig files".to_owned(),
        })
    }

    /// Adds a forward that did not start. Gives its number.
    fn add_forward(&self, state: &mut State, rule: ForwardRule, saved: bool) -> u64 {
        let id = state.forwards.next.wrapping_add(1);
        state.forwards.next = id;
        let stopped = ForwardView::new(rule, ForwardState::Stopped, WallClock::system().now());
        let last = Publisher::new(stopped, self.bin(), Notice::new()).read();
        state.forwards.entries.push(Entry { id, saved, running: None, last });
        id
    }

    /// Starts the forward of an entry, after the stop of its old run: the old listener closes
    /// first, so a fixed port binds again.
    fn run_forward(&self, state: &mut State, id: u64) {
        if state.phase == Phase::Stopped {
            return;
        }
        let Some(cluster) =
            state.forwards.entries.iter().find(|entry| entry.id == id).map(|e| e.rule().cluster.clone())
        else {
            return;
        };
        let parts = self.forward_parts(state);
        let feed = self.feed_of(state, &cluster);
        let Some(entry) = state.forwards.entries.iter_mut().find(|entry| entry.id == id) else { return };
        if let Some(old) = entry.running.take() {
            entry.last = old.stop();
        }
        entry.running = Some(Forward::start(entry.rule().clone(), feed, parts));
        self.repaint.request();
    }

    /// What a new forward gets: the settings of the program, with those of the settings file.
    fn forward_parts(&self, state: &mut State) -> ForwardParts {
        let settings = settings_of(&self.settings().forwards, &state.config);
        let forwards = &mut state.forwards;
        if forwards.watch.is_none() {
            let (changes, repaint) = (forwards.notice.clone(), self.repaint.clone());
            let always = Restart::Always(Backoff::default());
            forwards.watch = Some(self.tasks.start(TaskName::new("forward views"), always, move || {
                data::follow(vec![changes.changes()], repaint.clone())
            }));
        }
        ForwardParts {
            tasks: self.tasks.clone(),
            journal: self.journal.clone(),
            clock: WallClock::system(),
            bin: self.bin(),
            notice: forwards.notice.clone(),
            settings,
        }
    }

    /// The bin of the values that the readers held: one thread for the life of the core.
    fn bin(&self) -> crate::published::Bin {
        self.disposal.get_or_init(|| Disposal::start(&self.tasks)).bin()
    }
}

/// The settings of a new forward: those of the program, with the wait and the limit of the
/// settings file, which act at once.
fn settings_of(program: &ForwardSettings, config: &Config) -> ForwardSettings {
    let mut settings = program.clone();
    if config.set(Setting::ForwardWait).is_some() {
        settings.wait = config.duration(Setting::ForwardWait);
    }
    if config.set(Setting::ForwardConnections).is_some() {
        let limit = config.number(Setting::ForwardConnections);
        settings.connections = usize::try_from(limit).unwrap_or(usize::MAX);
    }
    settings
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::config::Value;
    use crate::config::table::{Form, spec};

    /// The wait and the limit of the file reach a new forward, a file with neither keeps those
    /// of the program, and the defaults of the table are those of the code.
    #[test]
    fn the_wait_and_the_limit_of_the_file_reach_a_new_forward() {
        let program = ForwardSettings { wait: Duration::from_secs(9), connections: 9, ..ForwardSettings::default() };
        assert_eq!(settings_of(&program, &Config::default()), program);
        let (mut config, mut defaults) = (Config::default(), Config::default());
        for (id, value) in [(Setting::ForwardWait, 7), (Setting::ForwardConnections, 3)] {
            config.values.insert(id, Value::Number(value));
            let Some(Form::Number(number)) = spec(id).map(|spec| spec.form) else { panic!("{id:?} is a number") };
            defaults.values.insert(id, Value::Number(number.default));
        }
        let settings = settings_of(&program, &config);
        assert_eq!((settings.wait, settings.connections), (Duration::from_secs(7), 3));
        assert_eq!(settings_of(&ForwardSettings::default(), &defaults), ForwardSettings::default());
    }
}
