use serde_json::json;

use super::*;

/// A Job as the server gives it: the generated selector and labels, an owner, a status.
fn served(manual_selector: bool) -> Json {
    let generated = json!({"batch.kubernetes.io/controller-uid": "u1", "batch.kubernetes.io/job-name": "once",
        "controller-uid": "u1", "job-name": "once", "team": "shop"});
    json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "once", "namespace": "shop", "uid": "u1", "resourceVersion": "42",
            "creationTimestamp": "2026-10-01T00:00:00Z", "generation": 1, "labels": generated.clone(),
            "annotations": {"note": "kept"}, "managedFields": [{"manager": "kubectl"}],
            "ownerReferences": [{"kind": "CronJob", "name": "nightly", "uid": "c1", "controller": true},
                {"kind": "CronJob", "name": "gone", "uid": "c2"}]},
        "spec": {"manualSelector": manual_selector, "backoffLimit": 2, "newerField": {"a": 1},
            "selector": {"matchLabels": {"batch.kubernetes.io/controller-uid": "u1"}},
            "template": {"metadata": {"labels": generated}, "spec": {"containers": [{"name": "c", "image": "busybox"}]}}},
        "status": {"succeeded": 1},
    })
}

#[test]
fn a_job_again_has_no_field_of_the_server_no_generated_label_and_only_the_owners_that_exist() {
    let manifest = again(&served(false), |owner| owner["uid"] == "c1").expect("a manifest");
    let expected = json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "once", "namespace": "shop", "labels": {"team": "shop"}, "annotations": {"note": "kept"},
            "ownerReferences": [{"kind": "CronJob", "name": "nightly", "uid": "c1", "controller": true}]},
        "spec": {"manualSelector": false, "backoffLimit": 2, "newerField": {"a": 1},
            "template": {"metadata": {"labels": {"team": "shop"}}, "spec": {"containers": [{"name": "c", "image": "busybox"}]}}},
    });
    assert_eq!(manifest, expected);
    let alone = again(&served(false), |_| false).expect("a manifest");
    assert_eq!(alone["metadata"].get("ownerReferences"), None, "no owner left, no empty list");
}

#[test]
fn a_job_with_its_own_selector_keeps_it_and_its_labels() {
    let manifest = again(&served(true), |_| true).expect("a manifest");
    assert_eq!(manifest["spec"]["selector"], served(true)["spec"]["selector"]);
    assert_eq!(manifest["spec"]["template"]["metadata"]["labels"]["controller-uid"], "u1");
    assert!(again(&json!("text"), |_| true).is_err());
    assert!(again(&json!({"spec": {}}), |_| true).is_err(), "no metadata");
}

#[test]
fn the_check_of_a_job_again_lets_the_server_make_a_name() {
    let check = for_check(&json!({"metadata": {"name": "once", "namespace": "shop"}, "spec": {}}));
    assert_eq!(check, json!({"metadata": {"generateName": "once-", "namespace": "shop"}, "spec": {}}));
}

#[test]
fn the_job_of_a_cron_job_is_the_job_of_kubectl_create_job_from() {
    let cron_job = json!({
        "metadata": {"name": "nightly", "namespace": "shop", "uid": "c1", "labels": {"own": "label"}},
        "spec": {"schedule": "0 1 * * *", "jobTemplate": {
            "metadata": {"labels": {"app": "report"}, "annotations": {"team": "shop"}},
            "spec": {"template": {"spec": {"restartPolicy": "Never", "containers": [{"name": "c", "image": "busybox"}]}}}}},
    });
    let job = of_cron_job(&cron_job, "nightly-manual-abcde").expect("a Job");
    let expected = json!({
        "apiVersion": "batch/v1", "kind": "Job",
        "metadata": {"name": "nightly-manual-abcde", "namespace": "shop", "labels": {"app": "report"},
            "annotations": {"cronjob.kubernetes.io/instantiate": "manual", "team": "shop"},
            "ownerReferences": [{"apiVersion": "batch/v1", "kind": "CronJob", "name": "nightly", "uid": "c1", "controller": true}]},
        "spec": {"template": {"spec": {"restartPolicy": "Never", "containers": [{"name": "c", "image": "busybox"}]}}},
    });
    assert_eq!(job, expected);
    let error = of_cron_job(&json!({"metadata": {"name": "x"}, "spec": {}}), "x-1").expect_err("no UID");
    assert_eq!(error.to_string(), "the CronJob has no UID");
}

#[test]
fn a_trigger_name_keeps_its_random_end_and_fits_a_job_name() {
    assert_eq!(name_with("nightly", "abcde"), "nightly-manual-abcde");
    let long = "n".repeat(70);
    let name = name_with(&long, "abcde");
    assert_eq!((name.len(), name.ends_with("-manual-abcde")), (63, true));
    let random = trigger_name("nightly");
    assert!(random.starts_with("nightly-manual-") && random.len() == "nightly-manual-".len() + 5);
    assert!(random.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'));
}
