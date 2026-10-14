//! The fake core: copies of the state that a test builds, and the commands that the views sent.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::CoreError;
use fresnel_core::app::{ClusterRow, Command, CoreView, Startup};
use fresnel_core::cluster::{ClusterId, LinkState};
use fresnel_core::journal::{Entry, JournalView, Level};
use fresnel_core::kubeconfig::AuthKind;
use fresnel_core::net::credential::{CredentialState, CredentialStatus};
use fresnel_core::net::task::{TaskReport, Tasks};
use fresnel_core::net::{
    ChannelState, ChannelStatus, ClusterStatus, ConnectStatus, ErrorClass, GroupStatus, ThrottleStatus, UpgradeStatus,
};
use fresnel_ui::{Clock, Shell};
use tokio::runtime::{Builder, Runtime};

pub(crate) const SECOND: Duration = Duration::from_secs(1);
/// The longest wait for a frame that must come.
const LIMIT: Duration = Duration::from_secs(30);
/// The most frames in sequence that ask for one more frame at once.
const MOST_FRAMES: u64 = 32;

/// The time of each view test: 2027-01-15 08:00:00 UTC.
pub(crate) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + 1_800_000_000 * SECOND
}

/// What stands in for the core: the copy that the next frame reads, each command that a
/// frame gave back, and the task start function with the report of each task that ended.
pub(crate) struct FakeCore {
    pub(crate) view: CoreView,
    pub(crate) sent: Vec<Command>,
    shell: Option<Shell>,
    tasks: Tasks,
    reports: Arc<Mutex<Vec<String>>>,
    /// A worker thread needs no runtime, but the task start function takes the handle of one.
    _runtime: Runtime,
}

impl FakeCore {
    fn new(view: CoreView) -> Self {
        let runtime = Builder::new_current_thread().build().expect("a runtime");
        let reports = Arc::new(Mutex::new(Vec::new()));
        let tasks = Tasks::new(runtime.handle().clone(), {
            let reports = Arc::clone(&reports);
            move |report: TaskReport| reports.lock().expect("a lock").push(report.to_string())
        });
        Self { view, sent: Vec::new(), shell: None, tasks, reports, _runtime: runtime }
    }
}

impl Drop for FakeCore {
    /// A worker thread of the views that ends is a defect, and it fails the test.
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(*self.reports.lock().expect("a lock"), Vec::<String>::new(), "a task of the views ended");
        }
    }
}

pub(crate) type Window = Harness<'static, FakeCore>;

/// The window of the app with no window: 1280 by 800 points in the dark theme.
pub(crate) fn window(view: CoreView) -> Window {
    window_with(view, egui::Theme::Dark, 1.0)
}

pub(crate) fn window_with(view: CoreView, theme: egui::Theme, pixels_per_point: f32) -> Window {
    build(view, theme, pixels_per_point, true)
}

/// The window with no GPU code, for a test that measures the time of its frames. Such a
/// window cannot give a picture.
pub(crate) fn window_with_no_gpu(view: CoreView, pixels_per_point: f32) -> Window {
    build(view, egui::Theme::Dark, pixels_per_point, false)
}

fn build(view: CoreView, theme: egui::Theme, pixels_per_point: f32, gpu: bool) -> Window {
    let frame = |ui: &mut egui::Ui, core: &mut FakeCore| {
        let shell = core.shell.get_or_insert_with(|| Shell::new(ui.ctx(), Clock::fixed(now()), &core.tasks));
        let commands = shell.show(ui, &core.view);
        core.sent.extend(commands);
    };
    let builder = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_pixels_per_point(pixels_per_point)
        .with_theme(theme)
        // A worker thread asks for a frame when its rows are there, at any moment. Some
        // frames in sequence are thus no fault; a view that asks with no end still is one.
        .with_max_steps(MOST_FRAMES)
        // The frame time of a display. With the longer one of the harness, a repaint that
        // a view asks for after 200 ms would count as a frame that follows at once.
        .with_step_dt(1.0 / 60.0);
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut window = builder.build_ui_state(frame, FakeCore::new(view));
    // The worker thread of the Pod table gives its first rows, also for no pod. The
    // count of the rows shows from then on: "25 pods", or "2 of 25 pods".
    until(&mut window, "the first rows of the Pod table", |window| {
        let is_count = |node: &egui_kittest::Node<'_>| {
            let text = node.value().unwrap_or_default();
            text.starts_with(|first: char| first.is_ascii_digit())
                && (text.ends_with(" pods") || text.ends_with(" pod"))
        };
        window.query_all_by_role(egui::accesskit::Role::Label).any(|node| is_count(&node))
    });
    window
}

/// Draws frames until the window meets the condition, as a window does that a worker
/// thread or the core wakes. The wait ends with the condition, not after a fixed time.
pub(crate) fn until(window: &mut Window, what: &str, met: impl Fn(&Window) -> bool) {
    let started = Instant::now();
    loop {
        window.run();
        if met(window) {
            return;
        }
        assert!(started.elapsed() < LIMIT, "in time: {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The window with the Connections view in its center.
pub(crate) fn connections_window(view: CoreView) -> Window {
    let mut window = window(view);
    open_page(&mut window, "Connections");
    window
}

/// Opens a page of the sidebar. The dock has a tab with the name of a page, so the role
/// tells the two.
pub(crate) fn open_page(window: &mut Window, page: &str) {
    window.get_by_role_and_label(egui::accesskit::Role::RadioButton, page).click();
    window.run();
}

/// Opens a tab of the dock.
pub(crate) fn open_tab(window: &mut Window, tab: &str) {
    window.get_by_role_and_label(egui::accesskit::Role::Button, tab).click();
    window.run();
}

/// True when the frame shows this text one time or more.
pub(crate) fn shows(window: &Window, text: &str) -> bool {
    window.query_all_by_label(text).next().is_some()
}

/// The texts of a list that the frame does not show.
pub(crate) fn absent<'a>(window: &Window, texts: &[&'a str]) -> Vec<&'a str> {
    texts.iter().copied().filter(|text| !shows(window, text)).collect()
}

/// A live channel with the values of a healthy connection.
pub(crate) fn channel(state: ChannelState) -> ChannelStatus {
    ChannelStatus {
        state,
        since: now() - 60 * SECOND,
        streams_open: 0,
        stream_budget: 70,
        server_stream_limit: Some(100),
        ping_time: Some(Duration::from_millis(12)),
        last_frame_age: Some(3 * SECOND),
        connect_attempts: 1,
        connections: 1,
        last_error: None,
    }
}

/// A channel that never got an answer.
pub(crate) fn new_channel(state: ChannelState) -> ChannelStatus {
    ChannelStatus { ping_time: None, last_frame_age: None, connections: 0, ..channel(state) }
}

pub(crate) fn group(channels: Vec<ChannelStatus>) -> GroupStatus {
    GroupStatus { channels, waiting: 0 }
}

/// The status of a cluster with these channels for watches, requests and log streams.
pub(crate) fn status(channels: [ChannelStatus; 3], credential: Option<CredentialState>) -> ClusterStatus {
    let [watches, requests, logs] = channels.map(|channel| group(vec![channel]));
    ClusterStatus {
        name: "fake".to_owned(),
        connect: ConnectStatus {
            addresses: Vec::new(),
            resolve_error: None,
            connect_time: Some(Duration::from_millis(40)),
            hedge_delay: Duration::from_millis(150),
            connects: 3,
            attempts: 3,
        },
        outage: None,
        watches,
        requests,
        logs,
        credential: credential.map(|state| CredentialStatus { state, plugin_runs: 1, unauthorized: 0, proofs: 0 }),
        upgrade: UpgradeStatus::default(),
        throttle: ThrottleStatus::default(),
    }
}

/// One live channel for each purpose.
pub(crate) fn live_channels() -> [ChannelStatus; 3] {
    [(); 3].map(|()| channel(ChannelState::Live))
}

/// A token that is valid for one more hour.
pub(crate) fn valid() -> CredentialState {
    CredentialState::Valid { until: Some(now() + 3600 * SECOND) }
}

/// A live cluster with a valid token.
pub(crate) fn live_status() -> ClusterStatus {
    status(live_channels(), Some(valid()))
}

/// A selected cluster with a plugin and this state of its connections.
pub(crate) fn row(name: &str, link: LinkState) -> ClusterRow {
    ClusterRow {
        id: ClusterId::new(name),
        server: Ok(format!("{name}.k8s.example:443")),
        auth: Some(AuthKind::Plugin("doctl".to_owned())),
        selected: true,
        link,
        data: None,
    }
}

pub(crate) fn connected(name: &str, status: ClusterStatus) -> ClusterRow {
    row(name, LinkState::On(Box::new(status)))
}

/// Gives a cluster the data of its stores.
pub(crate) trait WithData {
    fn with(self, data: Option<fresnel_core::app::ClusterData>) -> Self;
}

impl WithData for ClusterRow {
    fn with(self, data: Option<fresnel_core::app::ClusterData>) -> Self {
        Self { data, ..self }
    }
}

/// A cluster that is not selected.
pub(crate) fn off(name: &str) -> ClusterRow {
    ClusterRow { selected: false, ..row(name, LinkState::Off) }
}

pub(crate) fn error(text: &str) -> CoreError {
    CoreError::new(ErrorClass::Invalid, text)
}

/// The view of a core whose start is complete, with no saved table layout and an empty journal.
pub(crate) fn view(clusters: Vec<ClusterRow>) -> CoreView {
    let tables = Some(Arc::default());
    CoreView { startup: Startup::Ready, shell_problem: None, clusters, tables, journal: Arc::default() }
}

/// A journal with these entries, one for each second before now. `dropped` entries left it.
pub(crate) fn journal(entries: &[(Level, &str, &str)], dropped: u64) -> Arc<JournalView> {
    let count = entries.len() as u64;
    let entry = |(index, (level, source, text)): (u64, &(Level, &str, &str))| {
        Arc::new(Entry {
            number: dropped + index,
            time: now() - Duration::from_secs(count - index),
            level: *level,
            source: (*source).to_owned(),
            text: (*text).to_owned(),
        })
    };
    let of_level = |wanted: Level| entries.iter().filter(|(level, _, _)| *level == wanted).count() as u64;
    Arc::new(JournalView {
        entries: (0..).zip(entries).map(entry).collect(),
        total: dropped + count,
        warnings: of_level(Level::Warning),
        errors: of_level(Level::Error),
    })
}
