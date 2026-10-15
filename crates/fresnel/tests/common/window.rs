//! The window of the app on its real core, with no window of the system: each frame is
//! [`fresnel::frame::draw`], as in the app. The core starts after the first frame.

use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{Core, CoreParts, CoreSettings};
use fresnel_core::journal::{Journal, TASK_SOURCE};
use fresnel_core::net::WallClock;
use fresnel_testkit::Home;
use fresnel_ui::{Clock, Shell};
use tokio::runtime::Runtime;

/// The longest wait for a frame that must come.
pub(crate) const LIMIT: Duration = Duration::from_secs(60);

/// What a frame draws: the shell of the app on the core of the app.
pub(crate) struct App {
    pub(crate) core: Core,
    shell: Option<Shell>,
}

pub(crate) type Window = Harness<'static, App>;

/// The core of the app with the home folder of a test, as the app starts from Finder.
pub(crate) fn core(runtime: &Runtime, home: &Home) -> Core {
    Core::new(CoreParts {
        runtime: runtime.handle().clone(),
        base_env: home.finder_env(),
        journal: Journal::new(1000, WallClock::system()),
        state_file: home.path("state/state.json"),
        config_file: home.path("state/config.toml"),
        detectors: Vec::new(),
        settings: CoreSettings::default(),
    })
}

/// The window of the app with this size, and its core after the first frame.
pub(crate) fn window(core: Core, size: egui::Vec2) -> Window {
    let frame = |ui: &mut egui::Ui, app: &mut App| {
        let core = &app.core;
        let shell = app.shell.get_or_insert_with(|| Shell::new(ui.ctx(), Clock::system(), &core.tasks()));
        fresnel::frame::draw(shell, core, ui);
    };
    let builder = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).with_max_steps(64);
    let window = builder.build_ui_state(frame, App { core, shell: None });
    // The core starts after the first frame, as in the window of the app.
    let ctx = window.ctx.clone();
    window.state().core.start(move || ctx.request_repaint());
    window
}

/// Draws frames until the window meets the condition. Gives the time of the wait.
pub(crate) fn until(window: &mut Window, what: &str, met: impl Fn(&Window) -> bool) -> Duration {
    until_within(window, what, LIMIT, met)
}

/// Draws frames until the window meets the condition, for `limit` at most: a wait for a
/// cluster that starts a part of its own, such as metrics-server.
pub(crate) fn until_within(
    window: &mut Window,
    what: &str,
    limit: Duration,
    met: impl Fn(&Window) -> bool,
) -> Duration {
    let started = Instant::now();
    loop {
        window.run();
        if met(window) {
            return started.elapsed();
        }
        let journal = || window.state().core.view().journal;
        assert!(started.elapsed() < limit, "in time: {what}: {:#?}", journal().entries);
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(crate) fn shows(window: &Window, text: &str) -> bool {
    window.query_all_by_label(text).next().is_some()
}

/// Stops the core. No task of the app ended in the test: each fault was a state.
pub(crate) fn stop_with_no_ended_task(window: &Window) {
    let core = &window.state().core;
    let journal = core.view().journal;
    let ended: Vec<&str> =
        journal.entries.iter().filter(|entry| entry.source == TASK_SOURCE).map(|entry| entry.text.as_str()).collect();
    assert_eq!(ended, Vec::<&str>::new(), "no task of the app ended");
    core.stop();
}
