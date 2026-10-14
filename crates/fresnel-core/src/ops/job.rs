//! The manifests of "run again" of a Job and of "trigger now" of a CronJob (research 08
//! sections 5.1 and 5.3). Both work on the JSON of the server, so no field of a newer
//! server goes away.

use serde_json::{Map, Value as Json, json};

use crate::error::CoreError;

/// The labels that the server writes into a Job and its pod template with the UID of the
/// Job. The selector of a new Job would then not match its pods, and the server refuses it.
const GENERATED_LABELS: [&str; 4] =
    ["controller-uid", "batch.kubernetes.io/controller-uid", "job-name", "batch.kubernetes.io/job-name"];
/// The fields of the metadata that only the server sets.
const SERVER_FIELDS: [&str; 8] = [
    "uid",
    "resourceVersion",
    "creationTimestamp",
    "generation",
    "managedFields",
    "deletionTimestamp",
    "deletionGracePeriodSeconds",
    "selfLink",
];
/// The annotation of kubectl on a Job that a person made from a CronJob.
const INSTANTIATE: &str = "cronjob.kubernetes.io/instantiate";
/// The longest name of a Job: the server writes it into a label value.
const NAME_LIMIT: usize = 63;

/// The manifest of a Job for its create again, from the Job as the server gave it. An owner
/// reference stays only when `owner_exists` says that its owner has that UID now: the
/// garbage collector deletes a new Job whose owner is gone.
pub(super) fn again(job: &Json, owner_exists: impl Fn(&Json) -> bool) -> Result<Json, CoreError> {
    let mut manifest = job.clone();
    let object = manifest.as_object_mut().ok_or_else(|| CoreError::invalid("the Job is not a JSON object"))?;
    object.remove("status");
    let metadata = object.get_mut("metadata").and_then(Json::as_object_mut);
    let metadata = metadata.ok_or_else(|| CoreError::invalid("the Job has no metadata"))?;
    for field in SERVER_FIELDS {
        metadata.remove(field);
    }
    if let Some(owners) = metadata.get_mut("ownerReferences").and_then(Json::as_array_mut) {
        owners.retain(&owner_exists);
        if owners.is_empty() {
            metadata.remove("ownerReferences");
        }
    }
    if manifest.pointer("/spec/manualSelector") != Some(&Json::Bool(true)) {
        if let Some(spec) = manifest.get_mut("spec").and_then(Json::as_object_mut) {
            spec.remove("selector");
        }
        for labels in ["/metadata/labels", "/spec/template/metadata/labels"] {
            if let Some(labels) = manifest.pointer_mut(labels).and_then(Json::as_object_mut) {
                labels.retain(|key, _| !GENERATED_LABELS.contains(&key.as_str()));
            }
        }
    }
    Ok(manifest)
}

/// The manifest for the dry run of a create again. The old Job holds its name until its
/// delete, so the server makes up a name for the check; it stores nothing.
pub(super) fn for_check(manifest: &Json) -> Json {
    let mut check = manifest.clone();
    if let Some(metadata) = check.get_mut("metadata").and_then(Json::as_object_mut)
        && let Some(name) = metadata.remove("name")
    {
        metadata.insert("generateName".to_owned(), json!(format!("{}-", name.as_str().unwrap_or("job"))));
    }
    check
}

/// The Job of `kubectl create job <name> --from=cronjob/<cron job>`: the labels, the
/// annotations and the spec of the template of the CronJob, the mark of kubectl, and the
/// CronJob as its controller. kubectl sets no `blockOwnerDeletion`: that needs one more
/// permission on `cronjobs/finalizers`.
pub(super) fn of_cron_job(cron_job: &Json, name: &str) -> Result<Json, CoreError> {
    let read = |pointer: &str| cron_job.pointer(pointer).and_then(Json::as_str).filter(|text| !text.is_empty());
    let missing = |what: &str| CoreError::invalid(format!("the CronJob has no {what}"));
    let (cron_name, uid) =
        (read("/metadata/name").ok_or_else(|| missing("name"))?, read("/metadata/uid").ok_or_else(|| missing("UID"))?);
    let template = cron_job.pointer("/spec/jobTemplate").ok_or_else(|| missing("job template"))?;
    let mut annotations = Map::new();
    annotations.insert(INSTANTIATE.to_owned(), json!("manual"));
    if let Some(own) = template.pointer("/metadata/annotations").and_then(Json::as_object) {
        annotations.extend(own.clone());
    }
    let mut metadata = json!({
        "name": name,
        "annotations": annotations,
        "ownerReferences": [{"apiVersion": "batch/v1", "kind": "CronJob", "name": cron_name, "uid": uid, "controller": true}],
    });
    if let Some(namespace) = read("/metadata/namespace") {
        metadata["namespace"] = json!(namespace);
    }
    if let Some(labels) = template.pointer("/metadata/labels").filter(|labels| labels.is_object()) {
        metadata["labels"] = labels.clone();
    }
    let spec = template.get("spec").cloned().unwrap_or_else(|| json!({}));
    Ok(json!({"apiVersion": "batch/v1", "kind": "Job", "metadata": metadata, "spec": spec}))
}

/// A name for a Job of a CronJob: `<cron job>-manual-<5 letters>`, as Freelens makes it,
/// with the name of the CronJob cut so that the whole is a valid Job name.
pub fn trigger_name(cron_job: &str) -> String {
    let suffix: String = (0..5).map(|_| fastrand::alphanumeric().to_ascii_lowercase()).collect();
    name_with(cron_job, &suffix)
}

fn name_with(cron_job: &str, suffix: &str) -> String {
    let tail = format!("-manual-{suffix}");
    let base: String = cron_job.chars().take(NAME_LIMIT.saturating_sub(tail.len())).collect();
    format!("{base}{tail}")
}

#[cfg(test)]
mod tests;
