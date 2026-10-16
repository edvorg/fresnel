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
