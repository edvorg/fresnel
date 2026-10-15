//! The Operations tab (plan section 7.3): each state of a job (pending, done, failed, not
//! sent, result not known, cancelled), its steps with their marks and its error text, a
//! cancel while it runs, the count of the jobs that left the list, and the mark "change
//! pending" of a write.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::Color32;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::Command;
use fresnel_core::ops::{JobState, Operation, Outcome, Rollout, StepState};
use fresnel_core::store::Written;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::batch::v1::Job;

use super::scene::{allowed, job, jobs, open, target};
use crate::data::scene;
use crate::fake::{Window, error, open_tab, shows, view, whole, window, window_with};
use crate::kinds::in_panel;

/// A session with one job of each state, the newest last, on the Operations tab.
fn session() -> Window {
    session_window(1.0)
}

/// The same session at a scale of pixels.
pub(super) fn session_window(pixels_per_point: f32) -> Window {
    let deployments = scene::<Deployment>();
    let web = target("ams-prod", &deployments, "web");
    let broken = target("ams-prod", &deployments, "broken");
    let mut rollout =
        job(6, web.clone(), Operation::RolloutStatus, JobState::Pending, &[("follow the rollout", StepState::Running)]);
    Arc::make_mut(&mut rollout).report.outcome = Outcome::Rollout(Rollout::Waiting(
        "Waiting for deployment \"web\" rollout to finish: 1 of 2 updated replicas are available...".to_owned(),
    ));
    let list = vec![
        job(
            1,
            web.clone(),
            Operation::Scale(3),
            JobState::Done,
            &[("set the count of pods to 3", StepState::Done("the server stored it, version 812".to_owned()))],
        ),
        job(
            2,
            broken.clone(),
            Operation::Restart,
            JobState::Failed(error("can't restart paused deployment (run rollout resume first)")),
            &[("read the Deployment", StepState::Done("read".to_owned()))],
        ),
        job(3, web.clone(), Operation::Scale(4), JobState::NotSent(error("ams-prod is not connected")), &[]),
        job(
            4,
            web.clone(),
            Operation::Restart,
            JobState::Unknown(error("timeout: no answer in 15s")),
            &[
                ("restart each pod", StepState::Failed),
                (
                    "read the object, because the result of the write is not known",
                    StepState::Done("the object exists, version 815".to_owned()),
                ),
            ],
        ),
        job(
            5,
            web.clone(),
            Operation::RolloutStatus,
            JobState::Cancelled,
            &[("follow the rollout", StepState::Stopped)],
        ),
        rollout,
    ];
    let mut core = view(vec![allowed("ams-prod", &deployments)]);
    core.jobs = jobs(list, 7);
    let mut window = window_with(core, egui::Theme::Dark, pixels_per_point);
    open_tab(&mut window, "Operations");
    window
}

/// The numbers of the jobs whose rows are on the screen.
fn rows(window: &Window) -> Vec<u64> {
    let labels = window.query_all_by_label_contains("Job ").filter_map(|node| node.accesskit_node().label());
    labels.filter_map(|label| label.strip_prefix("Job ")?.parse().ok()).collect()
}

/// Clicks the row of a job. The list draws only the rows on the screen, the newest first,
/// so the wheel moves it to the row first.
fn choose(window: &mut Window, number: u64) {
    let label = format!("Job {number}");
    for _ in 0..10 {
        let shown = rows(window);
        if shown.contains(&number) {
            break;
        }
        // A newer job is higher up.
        let up = shown.iter().all(|shown| number > *shown);
        let title = window.get_by_label("Operation").rect();
        window.hover_at(egui::pos2(title.center().x, title.bottom() + 30.0));
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, if up { 48.0 } else { -48.0 }),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        window.event(wheel);
        window.run();
    }
    window.get_by_label(&label).scroll_to_me();
    window.run();
    window.get_by_label(&label).click();
    window.run();
}

/// The State column fits each state whole: "result not known" is the state of design rule 6.
#[test]
fn the_tab_lists_each_job_with_its_state_and_the_count_of_the_jobs_that_left() {
    let window = session();
    for state in ["done", "failed", "not sent", "result not known", "cancelled", "running"] {
        assert!(whole(&window, state), "the state {state}");
    }
    assert!(shows(&window, "1 running, 5 ended"));
    assert!(shows(&window, "7 older jobs that ended left the list"));
    assert!(shows(&window, "Restart") && shows(&window, "Deployment scene/broken"));
}

/// The Cluster column fits a name as wide as its old fixed column, so two contexts whose
/// names differ only at their end stay apart. The page shows an other cluster, so only the
/// job list names these two.
#[test]
fn the_cluster_column_shows_two_contexts_with_one_start_whole() {
    let deployments = scene::<Deployment>();
    let (east, west) = ("production-us-east", "production-us-west");
    let scale = |number: u64, cluster: &str| {
        job(number, target(cluster, &deployments, "web"), Operation::Scale(2), JobState::Done, &[])
    };
    let mut core = view(vec![allowed("ams-prod", &deployments)]);
    core.jobs = jobs(vec![scale(1, east), scale(2, west)], 0);
    let mut window = window(core);
    open_tab(&mut window, "Operations");
    for cluster in [east, west] {
        assert!(whole(&window, cluster), "the cluster {cluster}");
    }
}

/// The newest job shows first: a rollout that runs, with its step, its last state and a
/// cancel. The cancel is a command of the core.
#[test]
fn a_job_that_runs_shows_its_steps_and_its_cancel_sends_a_command() {
    let mut window = session();
    assert!(shows(&window, "Job 6: Rollout status"));
    assert!(shows(&window, "follow the rollout"));
    assert!(window.query_by_label_contains("1 of 2 updated replicas are available").is_some());
    window.get_by_label("Cancel job 6").click();
    window.run();
    assert_eq!(window.state().sent.last(), Some(&Command::Cancel(6)));
    choose(&mut window, 1);
    assert!(window.query_by_label("Cancel job 1").is_none(), "a job that ended has no cancel");
}

#[test]
fn each_job_that_ended_shows_its_state_its_error_text_and_its_steps() {
    let mut window = session();
    choose(&mut window, 1);
    assert!(shows(&window, "Job 1: Scale to 3") && shows(&window, "— the server stored it, version 812"));
    choose(&mut window, 2);
    assert!(window.query_by_label_contains("can't restart paused deployment").is_some(), "the error text");
    choose(&mut window, 3);
    assert!(window.query_by_label_contains("ams-prod is not connected").is_some());
    choose(&mut window, 4);
    assert!(window.query_by_label_contains("The app did not send it again").is_some(), "design rule 6");
    assert!(shows(&window, "— the object exists, version 815"));
    choose(&mut window, 5);
    assert!(shows(&window, "Job 5: Rollout status") && shows(&window, "cancelled"));
}

/// The text left of the text of a step on its line: the mark of its state.
fn mark_of(window: &Window, step: &str) -> String {
    let text = window.get_by_label(step).rect();
    let on_line = |rect: egui::Rect| (rect.center().y - text.center().y).abs() < 2.0 && rect.right() <= text.left();
    let marks = window.query_all_by_role(egui::accesskit::Role::Label).filter(|node| on_line(node.rect()));
    let mark = marks.max_by(|a, b| a.rect().right().total_cmp(&b.rect().right()));
    mark.and_then(|node| node.value()).expect("a mark left of the step")
}

/// True when each character of the text has a glyph of its own. egui draws a character
/// with no glyph as the square "◻", and `has_glyph` takes each glyph of the font of that
/// square for a missing one.
fn has_glyphs(window: &Window, text: &str) -> bool {
    let font = egui::TextStyle::Body.resolve(&window.ctx.global_style());
    let glyphs = |text: &str| {
        let galley = window.ctx.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE));
        galley.rows.iter().flat_map(|row| row.glyphs.iter().map(|glyph| glyph.uv_rect)).collect::<Vec<_>>()
    };
    let square = glyphs("◻");
    glyphs(text).iter().all(|glyph| !square.contains(glyph))
}

/// The mark of each state of a step has a glyph in the fonts of the app: "✓" had none.
#[test]
fn the_mark_of_each_state_of_a_step_has_a_glyph() {
    let mut window = session();
    let steps = [
        (6, "follow the rollout"),
        (4, "restart each pod"),
        (4, "read the object, because the result of the write is not known"),
        (5, "follow the rollout"),
    ];
    let mut marks = BTreeSet::new();
    for (number, step) in steps {
        choose(&mut window, number);
        marks.insert(mark_of(&window, step));
    }
    assert_eq!(marks.len(), 4, "running, failed, done and stopped: {marks:?}");
    let missing: Vec<&String> = marks.iter().filter(|mark| !has_glyphs(&window, mark)).collect();
    assert!(missing.is_empty(), "no glyph for {missing:?}");
    assert!(!has_glyphs(&window, "✓"), "the check sees a missing glyph");
}

/// "Run again" after the delete of the old Job keeps its manifest for a copy.
#[test]
fn a_run_again_whose_create_failed_offers_the_manifest() {
    let jobs_of_scene = scene::<Job>();
    let once = target("ams-prod", &jobs_of_scene, "once");
    let mut failed =
        job(9, once, Operation::RunAgain, JobState::Failed(error("conflict: jobs.batch \"once\" already exists")), &[]);
    Arc::make_mut(&mut failed).report.outcome = Outcome::Manifest("{\"kind\": \"Job\"}".into());
    let mut core = view(vec![allowed("ams-prod", &jobs_of_scene)]);
    core.jobs = jobs(vec![failed], 0);
    let mut window = window(core);
    open_tab(&mut window, "Operations");
    assert!(shows(&window, "The old Job is gone. Its manifest, for a create by hand:"));
    window.get_by_label("Copy the manifest").click();
    window.run();
}

/// Plan section 9.10: after a write, the object shows "change pending" until its watch
/// event arrives.
#[test]
fn the_panel_shows_change_pending_until_the_store_has_the_written_version() {
    let deployments = scene::<Deployment>();
    let web = deployments.iter().find(|object| object.metadata.name.as_deref() == Some("web")).expect("web");
    let stored: u64 =
        web.metadata.resource_version.as_deref().and_then(|version| version.parse().ok()).expect("a version");
    let mut meta = web.metadata.clone();
    meta.resource_version = Some((stored + 1).to_string());
    let mut scale = job(1, target("ams-prod", &deployments, "web"), Operation::Scale(3), JobState::Done, &[]);
    Arc::make_mut(&mut scale).report.outcome = Outcome::Written(Written::changed(&meta));
    let mut core = view(vec![allowed("ams-prod", &deployments)]);
    core.jobs = jobs(vec![scale], 0);
    let mut window = window(core);
    open(&mut window, "Deployments", "web");
    assert!(in_panel(&window, "Change pending: the server stored a change, and the watch did not bring it yet."));

    meta.resource_version = Some(stored.to_string());
    let mut done = job(1, target("ams-prod", &deployments, "web"), Operation::Scale(3), JobState::Done, &[]);
    Arc::make_mut(&mut done).report.outcome = Outcome::Written(Written::changed(&meta));
    window.state_mut().view.jobs = jobs(vec![done], 0);
    window.run();
    assert!(window.query_by_label_contains("Change pending").is_none(), "the store has that version");
}

/// The message of a refusal is a text of the server, up to 1 MiB, and a step can carry one
/// too: the tab shows the start of each, so a frame stays short.
#[test]
fn a_long_error_of_the_server_shows_its_start() {
    let deployments = scene::<Deployment>();
    let web = target("ams-prod", &deployments, "web");
    let message = "denied by the policy: ".to_owned() + &"y".repeat(1024 * 1024);
    let steps = [("restart each pod", StepState::Done(message.clone()))];
    let failed = job(1, web, Operation::Restart, JobState::Failed(error(&message)), &steps);
    let mut core = view(vec![allowed("ams-prod", &deployments)]);
    core.jobs = jobs(vec![failed], 0);
    let mut window = window(core);
    open_tab(&mut window, "Operations");
    let texts: Vec<String> =
        window.query_all_by_label_contains("denied by the policy").filter_map(|node| node.value()).collect();
    assert_eq!(texts.len(), 2, "the error and the step");
    for text in texts {
        assert!(text.chars().count() <= 2003, "{} characters", text.chars().count());
    }
}
