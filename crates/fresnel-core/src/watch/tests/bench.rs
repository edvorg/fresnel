//! One store on a source with a script: the test sets each answer of the server.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, TaskReport, Tasks};
use fresnel_net::{Signal, WallClock};
use tokio::runtime::Handle;
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::{Instant, timeout};

use crate::cluster::WatchEvent;
use crate::error::CoreError;
use crate::kinds::{Kind, Meta};
use crate::published::{Disposal, Held, Notice, Publisher, Reader};
use crate::store::{Snapshot, StoreState, Wait};
use crate::watch::lanes::{LaneStates, Lanes, Path};
use crate::watch::source::{Events, Page, Source};
use crate::watch::{ListGate, Parts, WatchSettings, run};

pub(super) const MS: Duration = Duration::from_millis(1);
pub(super) const SECOND: Duration = Duration::from_secs(1);
/// Longer than each wait of the loop. Paused time jumps to it when no signal arrives.
const LIMIT: Duration = Duration::from_secs(7200);
/// Each lane is live and the server takes requests.
pub(super) const LIVE: LaneStates =
    LaneStates { requests: Path::Live, watches: Path::Live, credential_failed: false, throttled_until: None };

/// The settings of the tests: pauses of 100, 200, 400 ms and then 3 s with no random part,
/// and a watch that the server ends after 60 s.
pub(super) fn settings() -> WatchSettings {
    let backoff = Backoff { first: 100 * MS, max: 3 * SECOND, jitter_percent: 0, reset_after: SECOND };
    WatchSettings { server_timeout: 60..=60, backoff, ..WatchSettings::default() }
}

/// An object with this name and resource version, in the namespace `shop`.
pub(super) fn object(name: &str, version: &str) -> Meta {
    let text = format!(
        r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"uid-{name}","resourceVersion":"{version}"}}}}"#
    );
    serde_json::from_str(&text).expect("an object")
}

/// One event of a watch, from the line that a server sends.
pub(super) fn event(kind: &str, name: &str, version: &str) -> WatchEvent<Meta> {
    let object = format!(
        r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"uid-{name}","resourceVersion":"{version}"}}}}"#
    );
    serde_json::from_str(&format!(r#"{{"type":"{kind}","object":{object}}}"#)).expect("an event")
}

/// The `ERROR` event of a server, with this code.
pub(super) fn error_event(code: u16, reason: &str, message: &str) -> WatchEvent<Meta> {
    let status = format!(r#"{{"kind":"Status","code":{code},"reason":"{reason}","message":"{message}"}}"#);
    serde_json::from_str(&format!(r#"{{"type":"ERROR","object":{status}}}"#)).expect("an event")
}

/// One answer of the script.
enum Answer<T> {
    Gives(Result<T, CoreError>),
    Panics(&'static str),
}

#[derive(Default)]
struct Lines {
    pages: VecDeque<Answer<Page<Meta>>>,
    watches: VecDeque<Answer<Feed>>,
    /// Each request of the loop, in words.
    calls: Vec<String>,
}

/// The answers of the server, in the sequence of the requests of one type. A request
/// with no answer in the script waits until the test gives one.
#[derive(Clone, Default)]
pub(super) struct Script {
    lines: Arc<Mutex<Lines>>,
    /// Wakes the requests that wait for an answer.
    added: Arc<Notify>,
}

/// The events of one open watch. The test sends each one, and its drop is the end of
/// the watch by the server.
pub(super) type Sender = mpsc::UnboundedSender<Result<WatchEvent<Meta>, CoreError>>;

pub(super) struct Feed(mpsc::UnboundedReceiver<Result<WatchEvent<Meta>, CoreError>>);

impl Events<Meta> for Feed {
    async fn next(&mut self) -> Option<Result<WatchEvent<Meta>, CoreError>> {
        self.0.recv().await
    }
}

impl Script {
    fn lines(&self) -> MutexGuard<'_, Lines> {
        self.lines.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn add_page(&self, answer: Answer<Page<Meta>>) {
        self.lines().pages.push_back(answer);
        self.added.notify_waiters();
    }

    fn add_watch(&self, answer: Answer<Feed>) {
        self.lines().watches.push_back(answer);
        self.added.notify_waiters();
    }

    /// Waits for the next answer that `take` finds in the script, and gives it.
    async fn answer<T>(&self, take: impl Fn(&mut Lines) -> Option<Answer<T>>) -> Result<T, CoreError> {
        loop {
            let added = self.added.notified();
            tokio::pin!(added);
            added.as_mut().enable();
            let next = take(&mut self.lines());
            match next {
                Some(Answer::Gives(result)) => return result,
                Some(Answer::Panics(message)) => panic!("{message}"),
                None => added.await,
            }
        }
    }

    /// The next list page has these objects. `version` is the resource version of the
    /// list, and `next` the token for one more page.
    pub(super) fn page(&self, objects: &[(&str, &str)], version: Option<&str>, next: Option<&str>) {
        let items = objects.iter().map(|(name, version)| object(name, version)).collect();
        let metadata = format!(r#"{{"resourceVersion":{},"continue":{}}}"#, json(version), json(next));
        let metadata = serde_json::from_str(&metadata).expect("the metadata of a list");
        self.add_page(Answer::Gives(Ok(Page { metadata, items: Some(items) })));
    }

    /// The next list page is one object with no name.
    pub(super) fn page_with_no_name(&self) {
        let metadata = serde_json::from_str(r#"{"resourceVersion":"7"}"#).expect("the metadata of a list");
        self.add_page(Answer::Gives(Ok(Page { metadata, items: Some(vec![Meta::default()]) })));
    }

    pub(super) fn list_fails(&self, error: CoreError) {
        self.add_page(Answer::Gives(Err(error)));
    }

    pub(super) fn list_panics(&self, message: &'static str) {
        self.add_page(Answer::Panics(message));
    }

    /// The next watch opens. The result sends its events.
    pub(super) fn watch_opens(&self) -> Sender {
        let (sender, events) = mpsc::unbounded_channel();
        self.add_watch(Answer::Gives(Ok(Feed(events))));
        sender
    }

    pub(super) fn watch_fails(&self, error: CoreError) {
        self.add_watch(Answer::Gives(Err(error)));
    }

    pub(super) fn watch_panics(&self, message: &'static str) {
        self.add_watch(Answer::Panics(message));
    }

    /// Each request so far, and the list is empty again.
    pub(super) fn take_calls(&self) -> Vec<String> {
        std::mem::take(&mut self.lines().calls)
    }
}

fn json(text: Option<&str>) -> String {
    text.map_or_else(|| "null".to_owned(), |text| format!("\"{text}\""))
}

impl Source<Meta> for Script {
    type Events = Feed;

    fn list_page(&self, limit: u32, token: Option<&str>) -> impl Future<Output = Result<Page<Meta>, CoreError>> + Send {
        let call = token.map_or_else(|| format!("list of {limit}"), |token| format!("list after {token}"));
        self.lines().calls.push(call);
        self.answer(|lines| lines.pages.pop_front())
    }

    fn watch(&self, from: &str, timeout_seconds: u32) -> impl Future<Output = Result<Feed, CoreError>> + Send {
        self.lines().calls.push(format!("watch after {from} for {timeout_seconds} s"));
        self.answer(|lines| lines.watches.pop_front())
    }
}

/// One store with its loop, on the paused clock.
pub(super) struct Bench {
    pub(super) script: Script,
    pub(super) reader: Reader<Snapshot<Meta>>,
    pub(super) lanes: watch::Sender<LaneStates>,
    pub(super) signal: Signal,
    pub(super) gate: Arc<ListGate>,
    pub(super) clock: WallClock,
    /// One line for each run of the task that ended.
    pub(super) reports: Arc<Mutex<Vec<String>>>,
    started: Instant,
    _task: TaskHandle,
    _disposal: Disposal,
}

impl Bench {
    /// Starts the loop of a Secret store. Each lane is live.
    pub(super) fn start(settings: WatchSettings) -> Self {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let tasks = Tasks::new(Handle::current(), {
            let reports = Arc::clone(&reports);
            move |report: TaskReport| reports.lock().expect("a lock").push(report.to_string())
        });
        let disposal = Disposal::start(&tasks);
        let started = Instant::now();
        let clock = WallClock::new(move || SystemTime::UNIX_EPOCH + started.elapsed());
        let first = Snapshot::empty(StoreState::Waiting(Wait::Channel), clock.now());
        let publisher = Publisher::new(first, disposal.bin(), Notice::new());
        let (script, lanes, signal) = (Script::default(), watch::Sender::new(LIVE), Signal::new());
        let gate = ListGate::new(settings.lists);
        let parts = Arc::new(Parts {
            kind: Kind::Secret,
            source: script.clone(),
            publisher: publisher.clone(),
            lanes: Lanes::new(lanes.subscribe()),
            signal: signal.clone(),
            gate: Arc::clone(&gate),
            settings: Arc::new(settings),
            clock: clock.clone(),
            max_items: None,
        });
        let restart = Backoff { first: 500 * MS, max: 500 * MS, jitter_percent: 0, reset_after: LIMIT };
        let name = TaskName::scoped("store", "bench/Secret");
        let task = tasks.start(name, Restart::Always(restart), move || run(Arc::clone(&parts)));
        Self {
            script,
            reader: publisher.reader(),
            lanes,
            signal,
            gate,
            clock,
            reports,
            started,
            _task: task,
            _disposal: disposal,
        }
    }

    /// Starts the loop and brings the store to its open watch: one list with the objects
    /// `a` and `b` at the version 7. Gives the sender of the watch.
    pub(super) async fn live(settings: WatchSettings) -> (Self, Sender) {
        let mut bench = Self::start(settings);
        bench.script.page(&[("a", "5"), ("b", "6")], Some("7"), None);
        let events = bench.script.watch_opens();
        bench.until_state("the open watch", StoreState::is_live).await;
        assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 7 for 60 s"]);
        (bench, events)
    }

    /// The time of the paused clock since the start of the bench.
    pub(super) fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Waits for a snapshot that meets the condition.
    pub(super) async fn until(&mut self, what: &str, met: impl Fn(&Snapshot<Meta>) -> bool) -> Held<Snapshot<Meta>> {
        let wait = async {
            loop {
                let snapshot = self.reader.read();
                if met(&snapshot) {
                    return snapshot;
                }
                self.reader.changed().await;
            }
        };
        let limited = timeout(LIMIT, wait).await;
        limited.unwrap_or_else(|_| panic!("in time: {what}; the state is `{}`", self.reader.read().state))
    }

    /// Waits for a state that meets the condition.
    pub(super) async fn until_state(&mut self, what: &str, met: impl Fn(&StoreState) -> bool) -> Held<Snapshot<Meta>> {
        self.until(what, |snapshot| met(&snapshot.state)).await
    }

    /// The name and the resource version of each object that the readers see.
    pub(super) fn objects(&self) -> Vec<(String, String)> {
        let snapshot = self.reader.read();
        let version = |meta: &Meta| meta.metadata.resource_version.clone().unwrap_or_default();
        snapshot.items.iter().map(|(key, object)| (key.name.to_string(), version(object))).collect()
    }

    pub(super) fn reports(&self) -> Vec<String> {
        self.reports.lock().expect("a lock").clone()
    }
}

/// True for the pause after this count of failures in sequence.
pub(super) fn is_pause(failures: u32) -> impl Fn(&StoreState) -> bool {
    move |state| matches!(state, StoreState::Backoff { failures: count, .. } if *count == failures)
}

/// The pairs of a test, as the type of [`Bench::objects`].
pub(super) fn pairs(objects: &[(&str, &str)]) -> Vec<(String, String)> {
    objects.iter().map(|(name, version)| ((*name).to_owned(), (*version).to_owned())).collect()
}
