use k8s_openapi::api::authorization::v1::{ResourceRule, SelfSubjectRulesReview, SubjectRulesReviewStatus};

use super::*;

fn rules(namespace: &str, verbs: &[&str]) -> Arc<Rules> {
    let rule = ResourceRule {
        verbs: verbs.iter().map(|verb| (*verb).to_owned()).collect(),
        api_groups: Some(vec!["*".to_owned()]),
        resources: Some(vec!["*".to_owned()]),
        resource_names: None,
    };
    let status = SubjectRulesReviewStatus { resource_rules: vec![rule], ..Default::default() };
    Arc::new(
        Rules::of(namespace, SelfSubjectRulesReview { status: Some(status), ..Default::default() }).expect("rules"),
    )
}

#[test]
fn an_action_is_decided_by_the_answer_of_a_review_first_and_then_by_the_rules_of_its_namespace() {
    let web = ObjectKey::new("shop", "web");
    let mut view = AccessView::default();
    let unread = view.decide(Action::Scale, Kind::Deployment, &web);
    assert_eq!(unread, Decision::Unknown("the rules are not read yet".into()));

    view.rules.insert("shop".into(), RulesState::Read(rules("shop", &["get", "list"])));
    let denied = view.decide(Action::Scale, Kind::Deployment, &web);
    assert_eq!(denied, Decision::Denied("no rule lets you patch deployments/scale (apps) web in shop".into()));
    assert_eq!(view.decide(Action::ViewManifest, Kind::Deployment, &web), Decision::Allowed);
    assert_eq!(view.decide(Action::CopyName, Kind::Deployment, &web), Decision::Allowed, "no request");

    let scale = access::needs(Action::Scale, Kind::Deployment, &web).remove(0);
    view.checks.insert(scale, CheckState::Answered(Decision::Allowed));
    assert_eq!(view.decide(Action::Scale, Kind::Deployment, &web), Decision::Allowed, "the server said yes");

    view.rules.insert("shop".into(), RulesState::Failed(CoreError::invalid("no answer")));
    let failed = view.decide(Action::Delete, Kind::Deployment, &web);
    assert_eq!(failed, Decision::Unknown("the read of the rules failed: no answer".into()));
}

#[tokio::test(start_paused = true)]
async fn an_answer_is_fresh_for_its_time_and_a_failure_never_is() {
    let failed = |state: &RulesState| matches!(state, RulesState::Failed(_));
    let read = (RulesState::Read(rules("shop", &["get"])), Instant::now());
    assert!(is_fresh(Some(&read), failed));
    assert!(is_fresh(Some(&(RulesState::Reading, Instant::now())), failed), "a review on its way");
    assert!(!is_fresh(Some(&(RulesState::Failed(CoreError::invalid("x")), Instant::now())), failed));
    assert!(!is_fresh(None, failed));
    tokio::time::advance(KEEP).await;
    assert!(!is_fresh(Some(&read), failed), "an answer of 5 minutes ago is old");
}

#[test]
fn a_refusal_drops_the_rules_and_the_answers_of_its_namespace_only() {
    let handle = AccessHandle { shared: Arc::default(), repaint: Repaint::new() };
    let now = Instant::now();
    let attributes = |namespace: &str| Attributes {
        verb: "delete",
        group: "",
        resource: "pods",
        subresource: "",
        namespace: namespace.into(),
        name: "web".into(),
    };
    {
        let mut known = handle.shared.known();
        for namespace in ["shop", "db"] {
            known.rules.insert(namespace.into(), (RulesState::Read(rules(namespace, &["*"])), now));
            known.checks.insert(attributes(namespace), (CheckState::Answered(Decision::Allowed), now));
        }
    }
    handle.forget("shop");
    let known = handle.shared.known();
    assert_eq!(known.rules.keys().map(|namespace| &**namespace).collect::<Vec<_>>(), ["db"]);
    assert_eq!(known.checks.keys().cloned().collect::<Vec<_>>(), [attributes("db")]);
}

#[tokio::test(start_paused = true)]
async fn the_answers_stay_inside_their_limit_and_the_oldest_answer_leaves_first() {
    let mut map: BTreeMap<u32, (RulesState, Instant)> = BTreeMap::new();
    keep(&mut map, 0, (RulesState::Reading, Instant::now()), RulesState::is_reading);
    for key in 1..=10 * KEPT_LIMIT as u32 {
        tokio::time::advance(Duration::from_millis(1)).await;
        keep(&mut map, key, (RulesState::Read(rules("shop", &["get"])), Instant::now()), RulesState::is_reading);
        assert!(map.len() <= KEPT_LIMIT, "{} answers", map.len());
    }
    assert!(map.contains_key(&0), "a review on its way stays");
    let first = 10 * KEPT_LIMIT as u32 - (KEPT_LIMIT as u32 - 2);
    assert_eq!(map.keys().nth(1), Some(&first), "the newest answers stay");
    keep(&mut map, first, (RulesState::Reading, Instant::now()), RulesState::is_reading);
    assert_eq!(map.len(), KEPT_LIMIT, "a new answer for a key that is there takes its place");
}
