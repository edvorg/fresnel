//! The save of one key of a Secret through `Command::ShowKeys` and `Command::SaveKeys`, as
//! the key view sends them: the save runs to its answer, the row shows "change pending"
//! until the watch event of the write, and only the cluster of the object changes.
//!
//! An assertion names keys only: a value never goes to the output of a test.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_core::app::{Command, CoreView, KeysView, ReadState};
use fresnel_core::keys::{self, Draft, KeyKind, WriteState};
use k8s_openapi::api::core::v1::Secret;

use crate::bed::{Bed, data, snapshot};
use crate::scene::field;

/// The key view of a cluster in the view of the core.
fn keys_of(view: &CoreView, cluster: &str) -> Option<Arc<KeysView>> {
    data(view, cluster)?.keys.clone()
}

pub(crate) async fn save_a_key_of_a_secret(bed: &Bed) {
    let of_a = field(&bed.a, "secret/creds", "{.data}").await;
    // The key view opens on the row of the Secret, with its UID.
    let row = bed.target::<Secret>("b", "creds");
    let target = keys::Target { cluster: row.cluster, kind: KeyKind::Secret, key: row.key, uid: row.uid };
    bed.core.send(Command::ShowKeys(Some(target)));
    let read = |view: &CoreView| keys_of(view, "b").is_some_and(|keys| keys.read == ReadState::Read);
    let view = bed.until("the read of creds in b", read).await;
    let object = keys_of(&view, "b").and_then(|keys| keys.object.clone()).expect("the object of the read");
    assert!(keys_of(&view, "a").is_none(), "the key view reads the object of b only");
    let before = snapshot::<Secret>(&view, "b").expect("the Secrets of b");

    let mut draft = Draft::new(object);
    draft.set("token", b"t-2".to_vec()).expect("a change");
    let save = draft.save().expect("a save");
    bed.core.send(Command::SaveKeys(save.clone()));
    let ended = |view: &CoreView| {
        let job = keys_of(view, "b").and_then(|keys| keys.save.clone());
        job.is_some_and(|job| job.state != WriteState::Pending)
    };
    let view = bed.until("the end of the save", ended).await;
    let job = keys_of(&view, "b").and_then(|keys| keys.save.clone()).expect("the save");
    assert!(job.save == save, "the save of the command");
    let WriteState::Done(answer) = &job.state else { panic!("a save that the server stored: {}", job.state) };
    let written = answer.written().expect("the identity of the answer");
    bed.mark::<Secret>("b", &before, &written).await;

    let token = field(&bed.b, "secret/creds", "{.data.token}").await;
    assert!(token == STANDARD.encode("t-2"), "the key token of b has the value of the save");
    assert!(field(&bed.b, "secret/creds", "{.data.user}").await == STANDARD.encode("admin"), "the key user of b stays");
    assert!(field(&bed.a, "secret/creds", "{.data}").await == of_a, "the Secret of a stays");

    bed.core.send(Command::ShowKeys(None));
    bed.until("the key view closes", |view| keys_of(view, "b").is_none()).await;
}
