//! The pictures of a panel: the metrics and the logs of the Deployment checkout-api of
//! prod-eu, a rollback of the release candidate on staging, and an edit of the keys of a
//! Secret. The filter of a page finds the object and is then empty again, so the table
//! shows the rows of each cluster.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::cluster::ClusterId;
use fresnel_core::keys::{KeyKind, Place, Target};
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{Color, LogLine, SourceState, SourceView, Span, Style};
use fresnel_core::ops::{JobState, Operation, Outcome, Revision};
use fresnel_core::store::ObjectKey;
use jiff::Timestamp;
use k8s_openapi::api::core::v1::Pod;
use serde_json::Value;

use super::objects::{DAY, HOUR, MINUTE, RESOURCE_VERSION, hash, suffix};
use super::world::{CHECKOUT, CHECKOUT_NEXT, PROD_EU, RC_REVISION, RC_ROLLOUT, STAGING, rollout};
use super::{TO_THE_END, save, target, uid_of, window};
use crate::fake::{Window, grow_dock, now, shows, until, widen_panel};
use crate::frame_time::wheel;
use crate::keys::scene::{give, loaded, read};
use crate::keys::{click, typed};
use crate::kinds::in_panel;
use crate::logs::{line, log_view, show, shown};
use crate::workloads::scene::{job, jobs, open};

/// The longest wait for the panel of an object.
const LIMIT: Duration = Duration::from_secs(30);
/// The revision of the release of checkout-api that prod runs.
const RELEASE: u32 = RC_REVISION - 2;
/// The panel of the metrics grows this much, near its widest, so the legend leaves each
/// chart its newest samples and each title of a chart fits on one line.
const METRICS_WIDENING: f32 = 200.0;
/// The panel of the rollback grows this much, so the list of the revisions fits.
const ROLLBACK_WIDENING: f32 = 100.0;
/// The dock with the logs grows this much, so the log takes most of the window.
const LOGS_DOCK: f32 = 250.0;
/// A place in the panel this far below its tabs, over its fields: the wheel there scrolls it.
const BELOW_TABS: egui::Vec2 = egui::vec2(20.0, 100.0);

/// Empties the filter of the page, so its table shows each row of the three clusters again;
/// `count` is the count of the rows then. The row of the panel stays selected.
fn clear_filter(window: &mut Window, count: &str) {
    window.get_by_role(Role::TextInput).click();
    window.run();
    window.key_press_modifiers(Modifiers::COMMAND, Key::A);
    window.key_press(Key::Backspace);
    until(window, count, |window| shows(window, count));
}

/// Opens the panel of checkout-api of a cluster. The filter leaves the row of checkout-api
/// of each cluster, and the panel of the first row opens; a click then goes to the row whose
/// Cluster cell names `cluster`.
fn checkout_of(window: &mut Window, cluster: &str) {
    open(window, "Deployments", "checkout-api");
    let started = Instant::now();
    while !in_panel(window, cluster) {
        assert!(started.elapsed() < LIMIT, "in time: the panel of checkout-api of {cluster}");
        let cells: Vec<f32> =
            window.query_all_by_role_and_label(Role::Label, cluster).map(|cell| cell.rect().center().y).collect();
        let rows = window.query_all_by_role_and_label(Role::Label, "checkout-api");
        if let Some(row) = rows.into_iter().find(|row| cells.iter().any(|y| (row.rect().center().y - y).abs() < 2.0)) {
            row.click();
        }
        window.run();
    }
    clear_filter(window, "19 Deployments");
}

/// The CPU and the memory of checkout-api in the last 30 minutes, with a line for each pod.
#[test]
fn the_metrics_of_a_deployment() {
    let mut window = window(egui::Theme::Dark);
    checkout_of(&mut window, PROD_EU.name);
    widen_panel(&mut window, METRICS_WIDENING);
    window.get_by_role_and_label(Role::Button, "Metrics").click();
    until(&mut window, "the charts", |window| shows(window, "CPU chart") && shows(window, "Memory chart"));
    save(&mut window, "deployment-metrics");
}

/// One revision of checkout-api: its number, version, age, cause and ReplicaSet.
fn revision(number: u32, (version, age, cause): (&str, u64, &str), replica_set: Option<String>) -> Revision {
    Revision {
        number: number.into(),
        name: replica_set.expect("the ReplicaSet of a Deployment"),
        created: Timestamp::try_from(now() - Duration::from_secs(age)).ok(),
        change_cause: Some(cause.to_owned()),
        images: vec![format!("shop/checkout-api:{version}")],
        current: number == RC_REVISION,
        template: Arc::new(Value::Null),
    }
}

/// The revisions of checkout-api of staging from a read of the server, and the dialog of a
/// rollback from the release candidate that crashes to the release that prod runs. The
/// revision before the current one is the candidate before, so the two choices differ.
#[test]
fn a_rollback_to_an_earlier_revision() {
    let mut window = window(egui::Theme::Dark);
    let candidate = format!("checkout-api-{}", suffix(&["1.43.0-rc.0"], 9));
    let current = rollout(&STAGING, &CHECKOUT_NEXT, RC_ROLLOUT);
    let revisions = vec![
        revision(RELEASE, ("1.42.3", 3 * DAY + 2 * HOUR + 41 * MINUTE, "release 1.42.3"), CHECKOUT.replica_set()),
        revision(
            RC_REVISION - 1,
            ("1.43.0-rc.0", DAY + 5 * HOUR + 9 * MINUTE, "candidate 1.43.0-rc.0"),
            Some(candidate),
        ),
        revision(RC_REVISION, ("1.43.0-rc.1", current, "candidate 1.43.0-rc.1"), CHECKOUT_NEXT.replica_set()),
    ];
    let checkout = target(STAGING.name, Kind::Deployment, "shop", "checkout-api");
    let mut history = job(1, checkout, Operation::History, JobState::Done, &[]);
    Arc::make_mut(&mut history).report.outcome = Outcome::Revisions(revisions.into());
    window.state_mut().view.jobs = jobs(vec![history], 0);
    checkout_of(&mut window, STAGING.name);
    widen_panel(&mut window, ROLLBACK_WIDENING);
    // The panel scrolls to its end, where the list of the revisions is.
    let tabs = window.get_by_role_and_label(Role::Button, "Overview").rect();
    window.hover_at(tabs.left_bottom() + BELOW_TABS);
    window.event(wheel(TO_THE_END));
    window.run();
    let button = format!("Roll back to {RELEASE}…");
    let dock = window.get_by_role_and_label(Role::Button, "Operations").rect().top();
    let shown = window.get_by_role_and_label(Role::Button, &button).rect();
    assert!(shown.top() > tabs.bottom() && shown.bottom() < dock, "{button} is in the panel: {shown:?}");
    window.get_by_role_and_label(Role::Button, &button).click();
    window.run();
    save(&mut window, "rollback");
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

/// The lines of the three pods of checkout-api in the last minute, about one each 1.5 s:
/// requests, one slow answer and one timeout of the payment provider.
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
        // A request comes at some point of its 1.5 s, so two lines stay in their sequence.
        let jitter = (hash(&["line", &number.to_string()]) % 900) as f64 / 1000.0;
        colored(61.0 - f64::from(number) * 1.5 - jitter, (number * 7 + number / 3) % 3, level, &text)
    };
    (0..40).map(line).collect()
}

/// The logs of the three pods of checkout-api, each one in its color, with a search.
#[test]
fn the_logs_of_a_deployment() {
    let mut window = window(egui::Theme::Dark);
    checkout_of(&mut window, PROD_EU.name);
    window.get_by_role_and_label(Role::Button, "Logs").click();
    window.run();
    let request = shown(&window).into_iter().next().expect("the request of the view");
    let of_checkout = |pod: &Pod| pod.metadata.name.as_deref().is_some_and(|name| name.starts_with("checkout-api-"));
    let pods = super::pods_of(&window, PROD_EU.name, of_checkout);
    let source = |pod: &String| SourceView {
        pod: ObjectKey::new("shop", pod),
        container: "checkout-api".into(),
        state: SourceState::Streaming,
        lines: 0,
        repeated: 0,
    };
    show(&mut window, vec![log_view(&request, pods.iter().map(source).collect(), checkout_lines())]);
    until(&mut window, "the lines", |window| window.query_by_label_contains("goes to the retry queue").is_some());
    grow_dock(&mut window, LOGS_DOCK);
    window.get_by_label("Search the log").click();
    window.get_by_label("Search the log").type_text("timeout");
    until(&mut window, "the hit", |window| shows(window, "1 hit"));
    save(&mut window, "deployment-logs");
}

/// The keys of a Secret with their values, one changed, and the dialog of the save.
#[test]
fn an_edit_of_the_keys_of_a_secret() {
    let mut window = window(egui::Theme::Dark);
    open(&mut window, "Secrets", "postgres-credentials");
    clear_filter(&mut window, "12 Secrets");
    let key = ObjectKey::new("payments", "postgres-credentials");
    let uid = uid_of(PROD_EU.name, Kind::Secret, "payments", "postgres-credentials");
    let target = Target { cluster: ClusterId::new(PROD_EU.name), kind: KeyKind::Secret, key, uid: uid.into() };
    let url = b"postgres://payments@postgres.payments.svc:5432/payments?sslmode=require\n";
    let values: &[(&str, Place, &[u8])] = &[
        ("password", Place::Data, b"example-password\n"),
        ("url", Place::Data, url),
        ("username", Place::Data, b"payments\n"),
    ];
    give(&mut window, PROD_EU.name, Some(loaded(read(&target, RESOURCE_VERSION, values))));
    click(&mut window, "Show the value of password");
    typed(&mut window, "Value of password", "example-password-2\n");
    click(&mut window, "Save 1 key…");
    click(&mut window, "Show the values");
    save(&mut window, "secret-save");
}
