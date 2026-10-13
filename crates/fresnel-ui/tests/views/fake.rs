//! The fake core: copies of the state that a test builds, and the commands that the views sent.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::CoreError;
use fresnel_core::app::{ClusterRow, Command, CoreView, Startup};
use fresnel_core::cluster::{ClusterId, LinkState};
use fresnel_core::journal::{Entry, JournalView, Level};
use fresnel_core::kubeconfig::AuthKind;
use fresnel_core::net::credential::{CredentialState, CredentialStatus};
use fresnel_core::net::{
    ChannelState, ChannelStatus, ClusterStatus, ConnectStatus, ErrorClass, GroupStatus, ThrottleStatus, UpgradeStatus,
};
use fresnel_ui::{Clock, Shell};

pub(crate) const SECOND: Duration = Duration::from_secs(1);

/// The time of each view test: 2027-01-15 08:00:00 UTC.
pub(crate) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + 1_800_000_000 * SECOND
}

/// What stands in for the core: the copy that the next frame reads, and each command that
/// a frame gave back.
pub(crate) struct FakeCore {
    pub(crate) view: CoreView,
    pub(crate) sent: Vec<Command>,
    shell: Option<Shell>,
}

pub(crate) type Window = Harness<'static, FakeCore>;

/// The window of the app with no window: 1280 by 800 points in the dark theme.
pub(crate) fn window(view: CoreView) -> Window {
    window_with(view, egui::Theme::Dark, 1.0)
}

pub(crate) fn window_with(view: CoreView, theme: egui::Theme, pixels_per_point: f32) -> Window {
    let core = FakeCore { view, sent: Vec::new(), shell: None };
    let frame = |ui: &mut egui::Ui, core: &mut FakeCore| {
        let shell = core.shell.get_or_insert_with(|| Shell::new(ui.ctx(), Clock::fixed(now())));
        let commands = shell.show(ui, &core.view);
        core.sent.extend(commands);
    };
    let mut window = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_pixels_per_point(pixels_per_point)
        .with_theme(theme)
        .wgpu()
        .build_ui_state(frame, core);
    window.run();
    window
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

/// A cluster that is not selected.
pub(crate) fn off(name: &str) -> ClusterRow {
    ClusterRow { selected: false, ..row(name, LinkState::Off) }
}

pub(crate) fn error(text: &str) -> CoreError {
    CoreError::new(ErrorClass::Invalid, text)
}

/// The view of a core whose start is complete, with an empty journal.
pub(crate) fn view(clusters: Vec<ClusterRow>) -> CoreView {
    CoreView { startup: Startup::Ready, shell_problem: None, clusters, journal: Arc::default() }
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
