//! The time of a frame of the Pods view with 10,000 pods (plan section 15, milestone M1): a
//! frame takes 16 ms or less during scroll, sort and filter.
//!
//! A frame is the pass of egui with its input, its layout and the accessibility tree of
//! the test, and then the triangles of its shapes. The upload to the GPU is not in the
//! time. The worker thread computes the rows, so a sort or a filter must not make a frame
//! long: the test draws frames with no pause while the worker computes.
//!
//! The limit holds in a release build:
//! `cargo test --release --package fresnel-ui --test views frame_time -- --nocapture`.
//! A debug build runs the same steps and checks no time, so each run of the tests proves
//! that 10,000 pods scroll, sort and filter.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Event, Modifiers, MouseWheelUnit, TouchPhase};
use egui_kittest::kittest::Queryable as _;
use k8s_openapi::api::core::v1::Pod;

use crate::data::{live, many_pods};
use crate::fake::{Window, shows, view, window_with_no_gpu};
use crate::pods::column;

/// The frame time of a display with 60 pictures in one second.
const LIMIT: Duration = Duration::from_millis(16);
const PODS: usize = 10_000;
/// The longest wait for the rows of the worker thread.
const ROWS_LIMIT: Duration = Duration::from_secs(60);

/// The times of the frames of one step of the test.
struct Step {
    name: &'static str,
    frames: Vec<Duration>,
}

impl Step {
    fn new(name: &'static str) -> Self {
        Self { name, frames: Vec::new() }
    }

    /// Draws one frame with the event that waits, and records its time.
    fn frame(&mut self, window: &mut Window) {
        let started = Instant::now();
        window.step();
        let output = window.output();
        let triangles = window.ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
        self.frames.push(started.elapsed());
        drop(triangles);
    }

    /// One frame for each event: a real window gets its events in this way.
    fn events(&mut self, window: &mut Window, events: impl IntoIterator<Item = Event>) {
        for event in events {
            window.event(event);
            self.frame(window);
        }
    }

    fn click(&mut self, window: &mut Window, pos: egui::Pos2) {
        let button = |pressed| Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.events(window, [Event::PointerMoved(pos), button(true), button(false)]);
    }

    /// Draws frames with no pause until the window meets the condition.
    fn until(&mut self, window: &mut Window, what: &str, met: impl Fn(&Window) -> bool) {
        let started = Instant::now();
        while !met(window) {
            assert!(started.elapsed() < ROWS_LIMIT, "{}: in time: {what}", self.name);
            self.frame(window);
        }
    }

    /// Prints the times of the step and checks the limit.
    fn check(&self) {
        let mut sorted = self.frames.clone();
        sorted.sort_unstable();
        let (median, longest) = (sorted[sorted.len() / 2], sorted[sorted.len() - 1]);
        eprintln!("frame time, {}: {} frames, median {median:.2?}, longest {longest:.2?}", self.name, sorted.len());
        if cfg!(debug_assertions) {
            return;
        }
        assert!(longest <= LIMIT, "{}: a frame took {longest:.2?}, the limit is {LIMIT:?}", self.name);
    }
}

fn title(window: &Window, title: &str) -> egui::Pos2 {
    window.get_by_role_and_label(egui::accesskit::Role::Button, title).rect().center()
}

fn first(window: &Window, title: &str) -> Option<String> {
    column(window, title).into_iter().next()
}

#[test]
fn a_frame_with_10_000_pods_takes_16_ms_or_less_during_scroll_sort_and_filter() {
    let all = many_pods(PODS);
    let mut window = window_with_no_gpu(view(vec![live("ams-prod", &all)]), 2.0);
    assert!(shows(&window, "10,000 pods"));
    let mut warm = Step::new("the first frames");
    (0..20).for_each(|_| warm.frame(&mut window));

    // Scroll: each frame moves the rows by more than two rows.
    let mut scroll = Step::new("scroll");
    let rows = title(&window, "Namespace") + egui::vec2(0.0, 200.0);
    scroll.events(&mut window, [Event::PointerMoved(rows)]);
    let wheel = |delta: f32| Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: egui::vec2(0.0, delta),
        phase: TouchPhase::Move,
        modifiers: Modifiers::NONE,
    };
    let top = first(&window, "Name");
    scroll.events(&mut window, (0..300).map(|_| wheel(-60.0)));
    assert_ne!(first(&window, "Name"), top, "the table did scroll");
    // egui spreads each move of the wheel over some frames, so the way back ends later.
    scroll.events(&mut window, (0..300).map(|_| wheel(60.0)));
    scroll.until(&mut window, "the first row again", |window| first(window, "Name") == top);
    scroll.check();

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
    sort.check();

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
    filter.check();

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
    stream.check();
}
