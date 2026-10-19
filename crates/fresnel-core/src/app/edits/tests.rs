use fresnel_net::env::ProcessEnv;
use fresnel_net::task::TaskReport;
use fresnel_net::{NetSettings, WallClock};
use tokio::runtime::Handle;

use super::*;
use crate::cluster::ClusterId;
use crate::kinds::Kind;
use crate::store::ObjectKey;

fn request(number: u64) -> EditRequest {
    let target =
        Target { cluster: ClusterId::new("a"), kind: Kind::Pod, key: ObjectKey::new("shop", "web"), uid: "u".into() };
    EditRequest { number, target }
}

fn read(number: u64, stage: Stage) -> Arc<EditRead> {
    let base = Arc::new(EditText { text: "kind: Pod\n".into(), version: Some("7".into()), lines: 1, decoded: None });
    let save =
        EditSave { editor: request(number), number: 1, base: Arc::clone(&base), text: "kind: Pod\n".into(), stage };
    let view = SaveView { save, state: SaveState::Pending };
    Arc::new(EditRead { request: request(number), state: EditState::Done(base), save: Some(view) })
}

/// A replace that waits or runs when the task starts again can be in effect: it ends with a
/// result that is not known and does not go again. A dry run waits for the new run.
#[test]
fn a_new_run_of_the_task_ends_a_pending_replace_as_not_known_and_keeps_a_dry_run() {
    let reads: Reads = Arc::new(Mutex::new(vec![read(1, Stage::Send), read(2, Stage::Check)]));
    end_after_restart(&reads);
    let states: Vec<SaveState> =
        lock(&reads).iter().filter_map(|read| read.save.as_ref()).map(|view| view.state.clone()).collect();
    let error = CoreError::invalid("the task of the editors started again during the save");
    assert_eq!(states, [SaveState::Unknown(error, None), SaveState::Pending]);
}

/// The view opens 8 editors at most; the core reads no more, also for a longer list.
#[tokio::test]
async fn the_core_reads_the_first_8_editors_of_a_longer_list() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
    let registry = Registry::new(tasks.clone(), Handle::current(), ProcessEnv::default(), NetSettings::default());
    let reader = EditReader::start(&tasks, registry, (Repaint::new(), Journal::new(10, WallClock::system())));
    assert_eq!(reader.show((1..=8).map(request).collect()), 0);
    assert_eq!(reader.show((1..=10).map(request).collect()), 2);
    let numbers: Vec<u64> = reader.reads().iter().map(|read| read.request.number).collect();
    assert_eq!((numbers, reader.wanted.borrow().len()), ((1..=8).collect(), EDITOR_LIMIT));
}
