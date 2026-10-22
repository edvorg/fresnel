//! The time of a frame of the Pods view with 10,000 pods (plan section 15, milestone M1): a
//! frame takes 16 ms or less during scroll, sort and filter.
//!
//! A frame is the pass of egui with its input, its layout and the accessibility tree of
//! the test, and then the triangles of its shapes. The upload to the GPU is not in the
//! time. The worker thread computes the rows, so a sort or a filter must not make a frame
//! long: the test draws frames with no pause while the worker computes.
//!
//! The limit holds in a release build on a known machine, the Mac on which it was set:
//! `cargo test --release --package fresnel-ui --test views frame_time -- --nocapture`.
//! A debug build runs the same steps and checks no time, so each run of the tests proves
//! that 10,000 pods scroll, sort and filter. CI runs them in a release build with the limits
//! of each runner in `FRESNEL_FRAME_LIMIT_MS` and `FRESNEL_SEARCH_LIMIT_MS` (see `limit`).
//! A step over its limit fails only when a second pass of its scene is over it too
//! (`crate::timing`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Event, Modifiers, MouseWheelUnit, TouchPhase};
use egui_kittest::kittest::Queryable as _;
use k8s_openapi::api::core::v1::Pod;

use crate::data::{changed, live, many_pods, pod};
use crate::fake::{Window, shows, view, window_with_no_gpu};
use crate::pods::column;
use crate::timing::{Pass, timed};

/// The frame time of a display with 60 pictures in one second.
const LIMIT: Duration = Duration::from_millis(16);
/// The variable with the frame time limit of a slower machine, in milliseconds.
const LIMIT_VARIABLE: &str = "FRESNEL_FRAME_LIMIT_MS";
const PODS: usize = 10_000;
/// The longest wait for the rows of the worker thread.
const ROWS_LIMIT: Duration = Duration::from_secs(60);

/// The times of the frames of one step of the test.
pub(crate) struct Step {
    name: &'static str,
    frames: Vec<Duration>,
}

impl Step {
    pub(crate) fn new(name: &'static str) -> Self {
        Self { name, frames: Vec::new() }
    }

    /// Draws one frame with the event that waits, and records its time.
    pub(crate) fn frame(&mut self, window: &mut Window) {
        let started = Instant::now();
        window.step();
        let output = window.output();
        let triangles = window.ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
        self.frames.push(started.elapsed());
        drop(triangles);
    }

    /// One frame for each event: a real window gets its events in this way.
    pub(crate) fn events(&mut self, window: &mut Window, events: impl IntoIterator<Item = Event>) {
        for event in events {
            window.event(event);
            self.frame(window);
        }
    }

    pub(crate) fn click(&mut self, window: &mut Window, pos: egui::Pos2) {
        let button = |pressed| Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.events(window, [Event::PointerMoved(pos), button(true), button(false)]);
    }

    /// Draws frames with no pause until the window meets the condition.
    pub(crate) fn until(&mut self, window: &mut Window, what: &str, met: impl Fn(&Window) -> bool) {
        let started = Instant::now();
        while !met(window) {
            assert!(started.elapsed() < ROWS_LIMIT, "{}: in time: {what}", self.name);
            self.frame(window);
        }
    }

    /// Prints the times of the step, and gives its longest frame with the limit to the pass,
    /// whose rule checks it (`crate::timing`).
    pub(crate) fn check(&self, pass: &mut Pass) {
        let mut sorted = self.frames.clone();
        sorted.sort_unstable();
        let (median, longest) = (sorted[sorted.len() / 2], sorted[sorted.len() - 1]);
        let (name, frames, label) = (self.name, sorted.len(), pass.label());
        eprintln!("frame time{label}, {name}: {frames} frames, median {median:.2?}, longest {longest:.2?}");
        pass.record(&format!("{name}, the longest frame"), longest, limit(LIMIT_VARIABLE, LIMIT));
    }
}

/// A time limit of a release build: `known` holds on the Mac on which it was set, and the
/// variable gives the limit of some other machine, in milliseconds. A shared CI runner is
/// slower, so the CI file sets it for each runner from the times that its runs print.
pub(crate) fn limit(variable: &str, known: Duration) -> Duration {
    match std::env::var(variable) {
        Ok(text) => Duration::from_millis(text.parse().unwrap_or_else(|_| panic!("{variable} is not a count: {text}"))),
        Err(std::env::VarError::NotPresent) => known,
        Err(error) => panic!("{variable}: {error}"),
    }
}

/// A move of the mouse wheel by this count of points.
pub(crate) fn wheel(delta: f32) -> Event {
    Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: egui::vec2(0.0, delta),
        phase: TouchPhase::Move,
        modifiers: Modifiers::NONE,
    }
}

pub(crate) fn title(window: &Window, title: &str) -> egui::Pos2 {
    window.get_by_role_and_label(egui::accesskit::Role::Button, title).rect().center()
}

pub(crate) fn first(window: &Window, title: &str) -> Option<String> {
    column(window, title).into_iter().next()
}

#[test]
fn a_frame_with_10_000_pods_stays_in_the_frame_limit_during_scroll_sort_and_filter() {
    timed(|pass| {
        let all = many_pods(PODS);
        let mut window = window_with_no_gpu(view(vec![live("ams-prod", &all)]), 2.0);
        assert!(shows(&window, "10,000 pods"));
        let mut warm = Step::new("the first frames");
        (0..20).for_each(|_| warm.frame(&mut window));

        // Scroll: each frame moves the rows by more than two rows.
        let mut scroll = Step::new("scroll");
        let rows = title(&window, "Namespace") + egui::vec2(0.0, 200.0);
        scroll.events(&mut window, [Event::PointerMoved(rows)]);
        let top = first(&window, "Name");
        scroll.events(&mut window, (0..300).map(|_| wheel(-60.0)));
        assert_ne!(first(&window, "Name"), top, "the table did scroll");
        // egui spreads each move of the wheel over some frames, so the way back ends later.
        scroll.events(&mut window, (0..300).map(|_| wheel(60.0)));
        scroll.until(&mut window, "the first row again", |window| first(window, "Name") == top);
        scroll.check(pass);

        // Sort: the worker computes the new sequence of 10,000 rows, and the frames go on.
        let mut sort = Step::new("sort");
        for (column_title, clicks, expected) in
            [("Restarts", 2, "2"), ("Age", 1, "35 s"), ("Status", 2, "Terminating"), ("Name", 1, "agent-8djvd-00002")]
        {
            for click in 1..=clicks {
                let pos = title(&window, column_title);
                sort.click(&mut window, pos);
                if click < clicks {
                    (0..30).for_each(|_| sort.frame(&mut window));
                }
            }
            let asked = Instant::now();
            sort.until(&mut window, &format!("the rows by {column_title}"), |window| {
                first(window, column_title).as_deref() == Some(expected)
            });
            eprintln!("frame time, sort: the rows by {column_title} show after {:.1?}", asked.elapsed());
        }
        sort.check(pass);

        // Filter: one frame for each character, and frames while the worker selects the rows.
        let mut filter = Step::new("filter");
        let field = window.get_by_label("Filter pods").rect().center();
        filter.click(&mut window, field);
        for character in "crash".chars() {
            filter.events(&mut window, [Event::Text(character.to_string())]);
            (0..10).for_each(|_| filter.frame(&mut window));
        }
        // Two of the 25 pods of the scene have "crash" in their name and their status.
        filter.until(&mut window, "the rows of the filter", |window| shows(window, "800 of 10,000 pods"));
        filter.check(pass);

        // A stream of snapshots during a scroll: 20 pods change before each frame.
        let mut stream = Step::new("scroll in a stream of snapshots");
        stream.events(&mut window, [Event::PointerMoved(rows)]);
        let mut now = all;
        for frame in 0..120_usize {
            for change in 0..20 {
                let index = (frame * 20 + change) * 7 % now.len();
                let mut pod = Pod::clone(&now[index]);
                pod.metadata.resource_version = Some(format!("{frame}"));
                now[index] = Arc::new(pod);
            }
            window.state_mut().view = view(vec![live("ams-prod", &now)]);
            stream.events(&mut window, [wheel(-24.0)]);
        }
        stream.until(&mut window, "the rows of the last snapshot", |window| {
            window.query_all_by_label_contains("of 10,000 pods").next().is_some()
        });
        stream.check(pass);
    });
}

/// The API gives most texts of an object no limit of their length. A status reason of
/// 1 MB took a frame of 63 ms each time its row came into view, and the panel of a pod
/// with 2,000 annotations two frames of 20 ms. Each view cuts such a text before its layout.
#[test]
fn a_frame_with_a_pod_of_extreme_values_stays_in_the_frame_limit() {
    timed(|pass| {
        let extreme = changed(&pod("db-0"), |value| {
            value["metadata"]["name"] = "extreme".into();
            value["metadata"]["uid"] = "the-uid-of-the-extreme-pod".into();
            value["status"]["reason"] = "R".repeat(1_000_000).into();
            let annotations: serde_json::Map<String, serde_json::Value> =
                (0..2000).map(|number| (format!("example.com/note-{number:04}"), "v".repeat(100).into())).collect();
            value["metadata"]["annotations"] = annotations.into();
            value["metadata"]["labels"]["long"] = "L".repeat(100_000).into();
        });
        let mut all = many_pods(1000);
        all.push(extreme);
        let mut window = window_with_no_gpu(view(vec![live("ams-prod", &all)]), 2.0);

        // The filter brings the row of the pod into the view.
        let mut table = Step::new("extreme values, the row");
        let field = window.get_by_label("Filter pods").rect().center();
        table.click(&mut window, field);
        table.events(&mut window, [Event::Text("extreme".to_owned())]);
        table.until(&mut window, "the row of the pod", |window| shows(window, "1 of 1,001 pods"));
        (0..20).for_each(|_| table.frame(&mut window));
        let status = column(&window, "Status").into_iter().next().expect("the status of the pod");
        assert!(status.starts_with("RRRR") && status.ends_with('…') && status.chars().count() < 500, "a cut text");
        table.check(pass);

        let mut panel = Step::new("extreme values, the panel");
        let row = window.get_by_label("extreme").rect().center();
        panel.click(&mut window, row);
        (0..20).for_each(|_| panel.frame(&mut window));
        assert!(shows(&window, "and 1,900 more"), "the panel shows the first annotations and the count of the rest");
        panel.check(pass);
    });
}
