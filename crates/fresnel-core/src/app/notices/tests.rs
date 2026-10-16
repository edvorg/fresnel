use std::sync::Arc;

use super::*;
use crate::app::JobView;
use crate::config::Value;
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::ops::{JobState, Operation, Order, Report, Target};
use crate::store::ObjectKey;

const SECOND: Duration = Duration::from_secs(1);

fn job(number: u64, state: JobState) -> Arc<JobView> {
    let key = ObjectKey::new("shop", "web");
    let target = Target { cluster: ClusterId::new("ams"), kind: Kind::Deployment, key, uid: "u".into() };
    let order = Order::new(target, Operation::Restart).expect("an order");
    Arc::new(JobView { number, order, report: Report { state, ..Report::default() } })
}

fn jobs(jobs: Vec<Arc<JobView>>) -> JobsView {
    JobsView { jobs, dropped: 0 }
}

fn look(jobs: &JobsView, health: Health, hidden: bool, now: Instant) -> Look<'_> {
    let reason = Some("watches: connect: Connection refused (os error 61)".to_owned());
    Look { jobs, clusters: vec![(ClusterId::new("ams"), health, reason)], failed_forwards: Vec::new(), hidden, now }
}

#[test]
fn each_failed_job_gives_one_notice() {
    let (config, now) = (Config::default(), Instant::now());
    let mut notices = Notices::default();
    let before = jobs(vec![job(1, JobState::Failed(CoreError::invalid("old")))]);
    assert_eq!(notices.step(&config, look(&before, Health::Live, false, now)).len(), 1);

    let refused = CoreError::invalid("deployments.apps \"web\" is forbidden");
    let after = jobs(vec![job(1, JobState::Failed(CoreError::invalid("old"))), job(2, JobState::NotSent(refused))]);
    let notice = Notice {
        title: "Restart failed in ams".to_owned(),
        body: "Deployment shop/web: deployments.apps \"web\" is forbidden".to_owned(),
    };
    assert_eq!(notices.step(&config, look(&after, Health::Live, false, now)), [notice], "also with the window open");
    assert_eq!(notices.step(&config, look(&after, Health::Live, false, now + SECOND)), [], "one notice for a job");
    let done = jobs(vec![job(3, JobState::Done), job(4, JobState::Cancelled)]);
    assert_eq!(notices.step(&config, look(&done, Health::Live, false, now)), [], "no notice for a job that ended well");
}

#[test]
fn a_cluster_that_stays_lost_while_the_window_is_hidden_gives_one_notice_for_each_loss() {
    let (config, start) = (Config::default(), Instant::now());
    let none = jobs(Vec::new());
    let mut notices = Notices::default();
    let at = |seconds: u64| start + Duration::from_secs(seconds);
    assert_eq!(notices.step(&config, look(&none, Health::Connecting, true, at(0))), [], "a first connect");
    assert_eq!(notices.step(&config, look(&none, Health::NoConnection, true, at(1))), [], "the loss starts");
    assert_eq!(notices.step(&config, look(&none, Health::NoConnection, false, at(70))), [], "the window shows it");
    let notice = Notice {
        title: "ams is lost for 70 s".to_owned(),
        body: "watches: connect: Connection refused (os error 61)".to_owned(),
    };
    assert_eq!(notices.step(&config, look(&none, Health::NoConnection, true, at(71))), [notice]);
    assert_eq!(notices.step(&config, look(&none, Health::NoCredential, true, at(200))), [], "one notice for a loss");

    // The cluster comes back, and a new loss gives a new notice after its own time.
    assert_eq!(notices.step(&config, look(&none, Health::Live, true, at(201))), []);
    assert_eq!(notices.step(&config, look(&none, Health::Failed, true, at(202))), []);
    assert_eq!(notices.step(&config, look(&none, Health::Failed, true, at(261))), [], "59 s");
    assert_eq!(notices.step(&config, look(&none, Health::Failed, true, at(262))).len(), 1, "60 s");
}

#[test]
fn the_settings_turn_each_notice_off_and_give_the_time_of_a_loss() {
    let mut config = Config::default();
    config.values.insert(Setting::NotifyFailed, Value::Flag(false));
    config.values.insert(Setting::LostAfter, Value::Number(10));
    let (start, mut notices) = (Instant::now(), Notices::default());
    let failed = jobs(vec![job(1, JobState::Unknown(CoreError::invalid("no answer")))]);
    assert_eq!(notices.step(&config, look(&jobs(Vec::new()), Health::NoConnection, true, start)), []);
    assert_eq!(notices.step(&config, look(&failed, Health::NoConnection, true, start + 10 * SECOND)).len(), 1);
    config.values.insert(Setting::NotifyLost, Value::Flag(false));
    let mut quiet = Notices::default();
    assert_eq!(quiet.step(&config, look(&failed, Health::NoConnection, true, start)), []);
    assert_eq!(quiet.step(&config, look(&failed, Health::NoConnection, true, start + 100 * SECOND)), []);
}

/// Plan section 11.6: a failed forward gives a notice, also with the window open, one for
/// each failure. A forward that reconnects gives none: it recovers by itself.
#[test]
fn each_failure_of_a_forward_gives_one_notice() {
    let (mut config, now, none) = (Config::default(), Instant::now(), jobs(Vec::new()));
    let rule = ForwardRule {
        cluster: ClusterId::new("ams"),
        kind: Kind::Service,
        key: ObjectKey::new("shop", "web"),
        port: 80,
        local: crate::forward::LocalPort::Fixed(8080),
        bind: crate::forward::Bind::Localhost,
    };
    let in_use = "the local port 8080 is in use: an other program listens on 127.0.0.1:8080".to_owned();
    let failed = |failed: Vec<(u64, ForwardRule, String)>| Look {
        failed_forwards: failed,
        ..look(&none, Health::Live, false, now)
    };
    let notice =
        Notice { title: "A forward failed in ams".to_owned(), body: format!("Service shop/web port 80: {in_use}") };
    let mut notices = Notices::default();
    assert_eq!(notices.step(&config, failed(vec![(1, rule.clone(), in_use.clone())])), std::slice::from_ref(&notice));
    assert_eq!(notices.step(&config, failed(vec![(1, rule.clone(), in_use.clone())])), [], "one notice for a failure");
    assert_eq!(notices.step(&config, failed(Vec::new())), [], "it started again");
    assert_eq!(notices.step(&config, failed(vec![(1, rule.clone(), in_use.clone())])), [notice], "a new failure");
    config.values.insert(Setting::NotifyForward, Value::Flag(false));
    assert_eq!(Notices::default().step(&config, failed(vec![(2, rule, in_use)])), [], "the setting turns it off");
}
