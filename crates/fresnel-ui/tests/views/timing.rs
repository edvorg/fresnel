//! The rule of the time limits of a release build: a scene with a step over its limit runs a
//! second time, and only a step that is over its limit in both passes fails.
//!
//! A shared CI runner stops the thread of a test now and then. On 2026-10-22 single frames of
//! 53 to 82 ms failed the macOS job three times, in steps whose median frame took 2 to 6 ms.
//! Such a stall comes in one pass; a frame that the code makes slow comes in each pass. The
//! longest frame stays the measure: a percentile would hide one long frame of the code, as
//! the first frame of a panel. Each pass prints its times.

use std::cell::Cell;
use std::time::Duration;

/// One time of a step against its limit: its longest frame, or the wait for the hits of a
/// search.
#[derive(Debug)]
struct Measure {
    name: String,
    time: Duration,
    limit: Duration,
}

impl Measure {
    fn is_over(&self) -> bool {
        self.time > self.limit
    }
}

/// The times of one pass of a scene.
#[derive(Debug, Default)]
pub(crate) struct Pass {
    second: bool,
    measures: Vec<Measure>,
}

impl Pass {
    /// Keeps the time of a step, with its limit. A step has one name in each pass.
    pub(crate) fn record(&mut self, name: &str, time: Duration, limit: Duration) {
        self.measures.push(Measure { name: name.to_owned(), time, limit });
    }

    /// The words that tell the pass in each printed time: none in the first pass.
    pub(crate) fn label(&self) -> &'static str {
        if self.second { ", second pass" } else { "" }
    }

    fn measure(&self, name: &str) -> Option<&Measure> {
        self.measures.iter().find(|measure| measure.name == name)
    }
}

/// Runs the scene of a time test with the rule of this module. A debug build runs it one
/// time and checks no time.
#[track_caller]
pub(crate) fn timed(scene: impl Fn(&mut Pass)) {
    if cfg!(debug_assertions) {
        scene(&mut Pass::default());
    } else if let Err(problem) = passes(scene) {
        panic!("{problem}");
    }
}

/// Runs the scene, and a second time when a step of it is over its limit. Gives each step
/// that is over its limit in both passes, with its two times.
fn passes(scene: impl Fn(&mut Pass)) -> Result<(), String> {
    let mut first = Pass::default();
    scene(&mut first);
    let over: Vec<&Measure> = first.measures.iter().filter(|measure| measure.is_over()).collect();
    if over.is_empty() {
        return Ok(());
    }
    let names: Vec<String> = over.iter().map(|measure| format!("{} ({:.2?})", measure.name, measure.time)).collect();
    eprintln!("over the limit in the first pass: {}; the scene runs a second time", names.join(", "));
    let mut second = Pass { second: true, measures: Vec::new() };
    scene(&mut second);
    let slow: Vec<String> = over
        .iter()
        .filter_map(|measure| {
            let again = second.measure(&measure.name);
            let (name, time) = (&measure.name, measure.time);
            match again {
                Some(again) if !again.is_over() => None,
                Some(again) => Some(format!(
                    "{name}: {time:.2?} in the first pass and {:.2?} in the second, the limit is {:?}",
                    again.time, again.limit
                )),
                None => Some(format!("{name}: {time:.2?} in the first pass, and the second has no such step")),
            }
        })
        .collect();
    if slow.is_empty() { Ok(()) } else { Err(slow.join("; ")) }
}

/// The limit of each step of the made-up scenes of the tests below.
const LIMIT: Duration = Duration::from_millis(50);

fn ms(count: u64) -> Duration {
    Duration::from_millis(count)
}

/// A scene of two steps whose times come from `times`, one pair for each pass. Gives the
/// result of the rule and the count of passes.
fn run(times: &[[u64; 2]]) -> (Result<(), String>, usize) {
    let count = Cell::new(0);
    let result = passes(|pass| {
        let [scroll, sort] = times[count.get()];
        count.set(count.get() + 1);
        pass.record("scroll", ms(scroll), LIMIT);
        pass.record("sort", ms(sort), LIMIT);
    });
    (result, count.get())
}

#[test]
fn a_scene_in_its_limits_runs_one_time() {
    assert_eq!(run(&[[50, 8]]), (Ok(()), 1));
}

#[test]
fn a_step_that_is_slow_in_each_pass_fails_with_both_times() {
    let (result, count) = run(&[[53, 8], [51, 8]]);
    assert_eq!(count, 2);
    assert_eq!(result, Err("scroll: 53.00ms in the first pass and 51.00ms in the second, the limit is 50ms".into()));
}

#[test]
fn one_long_frame_of_one_pass_does_not_fail() {
    assert_eq!(run(&[[70, 8], [6, 8]]), (Ok(()), 2));
}

/// Two stalls of two passes in two steps: each step was in its limit one time.
#[test]
fn a_long_frame_of_an_other_step_in_the_second_pass_does_not_fail() {
    assert_eq!(run(&[[70, 8], [6, 82]]), (Ok(()), 2));
}

#[test]
fn a_step_that_the_second_pass_does_not_have_fails() {
    let count = Cell::new(0);
    let result = passes(|pass| {
        count.set(count.get() + 1);
        if count.get() == 1 {
            pass.record("panel", ms(60), LIMIT);
        }
    });
    assert_eq!(result, Err("panel: 60.00ms in the first pass, and the second has no such step".into()));
}
