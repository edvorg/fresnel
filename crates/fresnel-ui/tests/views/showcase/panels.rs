//! The pictures of a panel: the metrics, the revisions and the logs of the Deployment
//! checkout-api of prod-eu, and an edit of the keys of a Secret.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Event, Modifiers, MouseWheelUnit, TouchPhase};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::cluster::ClusterId;
use fresnel_core::keys::{KeyKind, Place, Target};
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{Color, LogLine, SourceState, SourceView, Span, Style};
use fresnel_core::ops::{self, JobState, Operation, Outcome, Revision};
use fresnel_core::store::ObjectKey;
use jiff::Timestamp;
use k8s_openapi::api::core::v1::Pod;
use serde_json::Value;

use super::objects::{DAY, HOUR, suffix, uid};
use super::window;
use crate::fake::{Window, now, shows, until};
use crate::keys::scene::{give, loaded, read};
use crate::keys::{click, typed};
use crate::logs::{line, log_view, show, shown};
use crate::snapshots::save_clean;
use crate::workloads::scene::{job, jobs, open};

/// The window with the panel of the Deployment checkout-api of prod-eu: its row is the
/// first of the rows of the filter.
fn checkout(window: &mut Window) {
    open(window, "Deployments", "checkout-api");
}

/// The CPU and the memory of checkout-api in the last 30 minutes, with a line for each pod.
#[test]
fn the_metrics_of_a_deployment() {
    let mut window = window(egui::Theme::Dark);
    checkout(&mut window);
    window.get_by_role_and_label(Role::Button, "Metrics").click();
    until(&mut window, "the charts", |window| shows(window, "CPU chart") && shows(window, "Memory chart"));
    save_clean(&mut window, "deployment-metrics");
}

/// The revisions of checkout-api from a read of the server, and the dialog of a rollback.
#[test]
fn a_rollback_to_an_earlier_revision() {
    let mut window = window(egui::Theme::Dark);
    let revision = |number: i64, (version, age, cause): (&str, u64, &str), current| Revision {
        number,
        name: format!("checkout-api-{}", suffix(&["revision", version], 9)),
        created: Timestamp::try_from(now() - std::time::Duration::from_secs(age)).ok(),
        change_cause: Some(cause.to_owned()),
        images: vec![format!("shop/checkout-api:{version}")],
        current,
        template: Arc::new(Value::Null),
    };
    let revisions = vec![
        revision(5, ("1.42.0", 12 * DAY, "release 1.42.0"), false),
        revision(6, ("1.42.2", 5 * DAY + 3 * HOUR, "hotfix: retries"), false),
        revision(7, ("1.42.3", 2 * DAY + 4 * HOUR, "release 1.42.3"), true),
    ];
    let key = ObjectKey::new("shop", "checkout-api");
    let uid = uid(&["prod-eu", "Deployment", "shop", "checkout-api"]);
    let target = ops::Target { cluster: ClusterId::new("prod-eu"), kind: Kind::Deployment, key, uid: uid.into() };
    let mut history = job(1, target, Operation::History, JobState::Done, &[]);
    Arc::make_mut(&mut history).report.outcome = Outcome::Revisions(revisions.into());
    window.state_mut().view.jobs = jobs(vec![history], 0);
    checkout(&mut window);
    // The panel grows to the left, so that the list of the revisions fits in its width, the
    // dock grows, and the panel scrolls to its end: the list is then above the dialog.
    let tab = window.get_by_role_and_label(Role::Button, "Overview").rect();
    let edge = egui::pos2(tab.left() - 12.0, tab.bottom() + 100.0);
    super::move_edge(&mut window, edge, edge - egui::vec2(150.0, 0.0));
    let dock = super::dock_edge(&window);
    super::move_edge(&mut window, dock, dock - egui::vec2(0.0, 45.0));
    window.hover_at(edge + egui::vec2(200.0, 100.0));
    window.event(Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -2000.0),
        phase: TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    window.run();
    window.get_by_role_and_label(Role::Button, "Roll back to 6…").click();
    window.run();
    save_clean(&mut window, "rollback");
}

/// A line of a log with its level in the color of the program.
fn colored(seconds: f64, source: u32, (level, color): (&str, u8), text: &str) -> LogLine {
    let text = format!("{level:<5} {text}");
    let spans = [
        Span { start: 0, style: Style { foreground: Some(Color::Palette(color)), bold: true, ..Style::default() } },
        Span { start: level.len() as u32, style: Style::default() },
    ];
    LogLine { spans: Some(Arc::from(spans)), ..line(seconds, source, &text) }
}

/// The levels of the lines, with their colors of the 16 standard colors.
const INFO: (&str, u8) = ("INFO", 2);
const WARN: (&str, u8) = ("WARN", 3);
const ERROR: (&str, u8) = ("ERROR", 1);

/// The lines of the three pods of checkout-api in the last minute, one each 1.5 s: requests,
/// one slow answer and one timeout of the payment provider.
fn checkout_lines() -> Vec<LogLine> {
    let line = |number: u32| {
        let order = suffix(&["order", &number.to_string()], 6);
        let (level, text) = match number {
            17 => (WARN, "payment provider answered in 2840 ms; the budget is 2000 ms".to_owned()),
            29 => (ERROR, format!("payment provider timeout after 3000 ms; order ord_{order} goes to the retry queue")),
            _ if number.is_multiple_of(4) => {
                (INFO, format!("POST /api/v1/checkout 201 {}ms order=ord_{order}", 61 + number * 7 % 50))
            }
            _ if number % 4 == 2 => (INFO, format!("POST /api/v1/payments/intent 200 {}ms", 240 + number * 13 % 90)),
            _ => (INFO, format!("GET /api/v1/cart/crt_{order} 200 {}ms", 6 + number % 9)),
        };
        colored(61.0 - f64::from(number) * 1.5, (number * 7 + number / 3) % 3, level, &text)
    };
    (0..40).map(line).collect()
}

/// The logs of the three pods of checkout-api, each one in its color, with a search.
#[test]
fn the_logs_of_a_deployment() {
    let mut window = window(egui::Theme::Dark);
    checkout(&mut window);
    window.get_by_role_and_label(Role::Button, "Logs").click();
    window.run();
    let request = shown(&window).into_iter().next().expect("the request of the view");
    let of_checkout = |pod: &Pod| pod.metadata.name.as_deref().is_some_and(|name| name.starts_with("checkout-api-"));
    let pods = super::pods_of_prod_eu(&window, of_checkout);
    let source = |pod: &String| SourceView {
        pod: ObjectKey::new("shop", pod),
        container: "checkout-api".into(),
        state: SourceState::Streaming,
        lines: 0,
        repeated: 0,
    };
    show(&mut window, vec![log_view(&request, pods.iter().map(source).collect(), checkout_lines())]);
    until(&mut window, "the lines", |window| window.query_by_label_contains("goes to the retry queue").is_some());
    window.get_by_label("Search the log").click();
    window.get_by_label("Search the log").type_text("timeout");
    until(&mut window, "the hit", |window| shows(window, "1 hit"));
    save_clean(&mut window, "deployment-logs");
}

/// The keys of a Secret with their values, one changed, and the dialog of the save.
#[test]
fn an_edit_of_the_keys_of_a_secret() {
    let mut window = window(egui::Theme::Dark);
    open(&mut window, "Secrets", "postgres-credentials");
    let key = ObjectKey::new("payments", "postgres-credentials");
    let uid = uid(&["prod-eu", "Secret", "payments", "postgres-credentials"]);
    let target = Target { cluster: ClusterId::new("prod-eu"), kind: KeyKind::Secret, key, uid: uid.into() };
    let url = b"postgres://payments@postgres.payments.svc:5432/payments?sslmode=require\n";
    let values: &[(&str, Place, &[u8])] = &[
        ("password", Place::Data, b"example-password\n"),
        ("url", Place::Data, url),
        ("username", Place::Data, b"payments\n"),
    ];
    give(&mut window, "prod-eu", Some(loaded(read(&target, "48211", values))));
    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "example-password-2\n");
    click(&mut window, "Save 1 key…");
    click(&mut window, "Show the values");
    save_clean(&mut window, "secret-save");
}
