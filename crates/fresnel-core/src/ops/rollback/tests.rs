use serde_json::json;

use super::super::history::tests::{deployment as of_image, set, stored, template};
use super::super::request::tests::{parts, target};
use super::*;
use crate::kinds::Kind;

fn sets() -> Vec<Json> {
    vec![
        set("web-1", "1", "web:1", "2026-10-01T00:00:00Z", "dep"),
        set("web-2", "2", "web:2", "2026-10-02T00:00:00Z", "dep"),
        set("web-3", "3", "web:3", "2026-10-03T00:00:00Z", "dep"),
        set("foreign", "8", "web:8", "2026-10-03T00:00:00Z", "other"),
    ]
}

fn patch_of(plan: Plan) -> (String, String, String, Json) {
    match plan {
        Plan::Patch(request) => parts(&request),
        Plan::Skip(text) => panic!("a patch, not a skip: {text}"),
    }
}

#[test]
fn a_deployment_goes_back_with_the_template_and_the_annotations_of_its_replica_set() {
    let web = target(Kind::Deployment, "shop", "web");
    let own = json!({"deployment.kubernetes.io/revision": "3", "team": "shop",
        "kubectl.kubernetes.io/last-applied-configuration": "{}"});
    let now = of_image("web:3", own);
    let (method, url, content, body) = patch_of(deployment(&web, &now, &sets(), 0).expect("a plan"));
    assert_eq!((method.as_str(), content.as_str()), ("PATCH", "application/json-patch+json"));
    assert_eq!(url, "/apis/apps/v1/namespaces/shop/deployments/web?&fieldManager=fresnel");
    // Revision 0 is revision 2: the template without the hash label, the kept annotations of
    // the Deployment and the other annotations of the ReplicaSet. "team" goes, as in kubectl.
    let annotations = json!({"deployment.kubernetes.io/revision": "3",
        "kubectl.kubernetes.io/last-applied-configuration": "{}", "kubernetes.io/change-cause": "set web:2"});
    let expected = json!([
        {"op": "replace", "path": "/spec/template", "value": template("web:2", None)},
        {"op": "replace", "path": "/metadata/annotations", "value": annotations},
    ]);
    assert_eq!(body, expected);

    let first = patch_of(deployment(&web, &now, &sets(), 1).expect("a plan")).3;
    assert_eq!(first[0]["value"], template("web:1", None));
    let bare =
        patch_of(deployment(&web, &json!({"metadata": {"uid": "dep"}, "spec": {}}), &sets(), 1).expect("a plan"));
    assert_eq!(bare.3[1]["op"], "add", "an object with no annotations gets them with `add`");
}

#[test]
fn a_deployment_refuses_a_rollback_that_kubectl_refuses_and_skips_the_current_template() {
    let web = target(Kind::Deployment, "shop", "web");
    let now = of_image("web:3", json!({}));
    let text = |result: Result<Plan, CoreError>| result.expect_err("an error").to_string();
    assert_eq!(text(deployment(&web, &now, &sets(), 8)), "unable to find specified revision 8 in history");
    assert_eq!(text(deployment(&web, &now, &sets(), -1)), "unable to find specified revision -1 in history");
    let one = [set("web-1", "1", "web:1", "2026-10-01T00:00:00Z", "dep")];
    assert_eq!(text(deployment(&web, &now, &one, 0)), "no rollout history found for deployment \"web\"");
    let mut paused = now.clone();
    paused["spec"]["paused"] = json!(true);
    let refusal =
        "you cannot rollback a paused deployment; resume it first with 'kubectl rollout resume' and try again";
    assert_eq!(text(deployment(&web, &paused, &sets(), 2)), refusal);
    match deployment(&web, &now, &sets(), 3).expect("a plan") {
        Plan::Skip(text) => assert_eq!(text, "skipped rollback (current template already matches revision 3)"),
        Plan::Patch(_) => panic!("the template of revision 3 is the current one"),
    }
}

#[test]
fn a_stateful_set_goes_back_with_the_data_of_its_controller_revision() {
    let db = target(Kind::StatefulSet, "shop", "db");
    let all = [stored("db-1", 1, "db:1"), stored("db-3", 3, "db:3"), stored("db-2", 2, "db:2")];
    let now = of_image("db:3", json!({}));
    let (method, url, content, body) = patch_of(controller(&db, &now, &all, 0).expect("a plan"));
    assert_eq!((method.as_str(), content.as_str()), ("PATCH", "application/strategic-merge-patch+json"));
    assert_eq!(url, "/apis/apps/v1/namespaces/shop/statefulsets/db?&fieldManager=fresnel");
    assert_eq!(&body, &all[2]["data"], "revision 0 is the second highest: 2");
    assert_eq!(body["spec"]["template"]["$patch"], "replace");
    let first = patch_of(controller(&db, &now, &all, 1).expect("a plan")).3;
    assert_eq!(&first, &all[0]["data"]);

    match controller(&db, &now, &all, 3).expect("a plan") {
        Plan::Skip(text) => assert_eq!(text, "skipped rollback (current template already matches revision 3)"),
        Plan::Patch(_) => panic!("revision 3 has the current template"),
    }
    let error = controller(&db, &now, &all[..1], 0).expect_err("one revision");
    assert_eq!(error.to_string(), "no last revision to roll back to");
    let error = controller(&db, &now, &all, 9).expect_err("no revision 9");
    assert_eq!(error.to_string(), "unable to find specified revision 9 in history");
}
