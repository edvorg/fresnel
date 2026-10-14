//! The history and the rollback of a workload. Both read the workload and the objects with
//! its revisions from the server: a ControllerRevision has no data in its store, and a
//! rollback must start from the newest object, as kubectl does.

use serde_json::Value as Json;

use super::super::rollback::{self, Plan};
use super::super::{Target, history};
use super::{JobState, Outcome, Parts, Progress, Steps, read, read_failure, write};
use crate::error::CoreError;
use crate::kinds::Kind;

/// Reads the revisions of a workload. The outcome has them.
pub(super) async fn history<P: Progress>(
    target: &Target,
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<(), JobState> {
    let (workload, items) = workload_and_revisions(target, parts, job).await?;
    let revisions = match target.kind {
        Kind::Deployment => history::of_deployment(&workload, &items),
        _ => history::of_controller(&workload, &items),
    };
    job.outcome(Outcome::Revisions(revisions.into()));
    Ok(())
}

/// Goes back to revision `to`, or does nothing when the workload has its template already.
pub(super) async fn rollback<P: Progress>(
    target: &Target,
    parts: &Parts,
    job: &mut Steps<'_, P>,
    to: i64,
) -> Result<(), JobState> {
    let (workload, items) = workload_and_revisions(target, parts, job).await?;
    job.begin("find the revision");
    let plan = match target.kind {
        Kind::Deployment => rollback::deployment(target, &workload, &items, to),
        _ => rollback::controller(target, &workload, &items, to),
    };
    match plan.map_err(JobState::Failed)? {
        Plan::Skip(text) => {
            job.done(text);
            Ok(())
        }
        Plan::Patch(request) => {
            job.done("found");
            let text = if to == 0 {
                "roll back to the revision before".to_owned()
            } else {
                format!("roll back to revision {to}")
            };
            write(parts, job, text, Ok(*request)).await
        }
    }
}

/// The workload and the items of the list of its revisions, as the server gave them.
async fn workload_and_revisions<P: Progress>(
    target: &Target,
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<(Json, Vec<Json>), JobState> {
    let workload = read(parts, job, "read the workload", super::super::request::get(target)).await?;
    let uid = workload.pointer("/metadata/uid").and_then(Json::as_str);
    if !target.uid.is_empty() && uid != Some(&*target.uid) {
        return Err(JobState::Failed(CoreError::invalid(
            "the object of this name is an other object now: it has an other UID",
        )));
    }
    job.begin("list its revisions");
    let request = history::list(target, &workload).map_err(JobState::Failed)?;
    let list: Json = parts.gateway.list_page(request).await.map_err(read_failure)?;
    let items = list.get("items").and_then(Json::as_array).cloned().unwrap_or_default();
    job.done(format!("{} objects", items.len()));
    Ok((workload, items))
}
