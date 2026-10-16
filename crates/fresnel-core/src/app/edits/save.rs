//! The saves of the editors (plan section 12.9): a dry run, then the replace with the
//! resource version of the read, and the merge after a conflict.
//!
//! The view sends a save two times: once as a check, a dry run whose answer the diff before
//! the save shows, and once to send it, after the user saw that diff. When the server refuses
//! the replace with HTTP 409, the object changed since the read: the save reads it again and
//! merges the changes of the user with the changes of the server. The view then shows the
//! merge, and the user saves again, so nothing goes a second time without a question. After
//! a replace whose result is not known, the save reads the object in the same way (design
//! rule 6): when the write is in effect, the merge has the text of the user.

use std::fmt;
use std::sync::Arc;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::EditRequest;
use crate::cluster::{Fate, Gateway, Registry, WriteError};
use crate::error::CoreError;
use crate::manifest::merge::{self, Merge};
use crate::manifest::{EditText, read_for_edit, save};
use crate::store::Written;

/// A save of the text of an editor.
#[derive(Clone, PartialEq, Eq)]
pub struct EditSave {
    /// The request of the editor.
    pub editor: EditRequest,
    /// A number of the view that grows with each save of the editor.
    pub number: u64,
    /// The read that the text starts from: the replace names its version, and a merge takes
    /// its text as the base.
    pub base: Arc<EditText>,
    /// The text of the editor.
    pub text: Arc<str>,
    pub stage: Stage,
}

impl fmt::Debug for EditSave {
    /// The size of the text, never the text: the text of a Secret has its values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EditSave")
            .field("editor", &self.editor)
            .field("number", &self.number)
            .field("version", &self.base.version)
            .field("bytes", &self.text.len())
            .field("stage", &self.stage)
            .finish()
    }
}

/// What a save asks of the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// A dry run: the server checks the object and stores nothing.
    Check,
    /// The replace.
    Send,
}

/// The last save of an editor, with its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveView {
    pub save: EditSave,
    pub state: SaveState,
}

impl SaveView {
    /// The result of a replace that the server stored, for the mark "change pending" of its
    /// object while its watch event did not arrive (plan section 9.10).
    pub fn written(&self) -> Option<Written> {
        let SaveState::Saved(answer) = &self.state else {
            return None;
        };
        let target = &self.save.editor.target;
        Written::changed(&ObjectMeta {
            name: Some(target.key.name.to_string()),
            namespace: Some(target.key.namespace.to_string()).filter(|namespace| !namespace.is_empty()),
            uid: Some(target.uid.to_string()),
            resource_version: answer.version.clone(),
            ..ObjectMeta::default()
        })
    }
}

/// How far a save is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    /// The request is on its way.
    Pending,
    /// The dry run passed: the object as the server would store it.
    Checked(Arc<EditText>),
    /// The server stored the object: the text of its answer is the new read of the editor.
    Saved(Arc<EditText>),
    /// The object changed on the server since the read (HTTP 409), and nothing changed: its
    /// new read and the merge.
    Conflict(Arc<Remote>),
    /// The parser or the server refused the save, and nothing changed.
    Failed(CoreError),
    /// The request did not go to the server.
    NotSent(CoreError),
    /// The replace went out and no answer came back: it can be in effect. The read after
    /// it and its merge, when that read worked.
    Unknown(CoreError, Option<Arc<Remote>>),
}

/// The object on the server after a conflict or a write with an unknown result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    /// The new read.
    pub read: Arc<EditText>,
    /// The merge of the text of the user with the new read, on the base of the save.
    pub merge: Merge,
}

/// The words of the state for the journal. They name no value.
impl fmt::Display for SaveState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => f.write_str("on its way"),
            Self::Checked(_) => f.write_str("checked"),
            Self::Saved(text) => write!(f, "done, version {}", text.version.as_deref().unwrap_or("none")),
            Self::Conflict(_) => f.write_str("a conflict: the object changed since the read"),
            Self::Failed(error) => write!(f, "refused: {error}"),
            Self::NotSent(error) => write!(f, "not sent: {error}"),
            Self::Unknown(error, _) => write!(f, "its result is not known: {error}"),
        }
    }
}

/// "the save of Deployment shop/web": the start of a journal entry. It names no value.
pub(super) fn save_of(save: &EditSave) -> String {
    let target = &save.editor.target;
    let (kind, key) = (target.kind.info().name, &target.key);
    let name = if key.namespace.is_empty() { key.name.to_string() } else { format!("{}/{}", key.namespace, key.name) };
    format!("the save of {kind} {name}")
}

/// Runs a save through the gateway of the cluster of its editor.
pub(super) async fn run(registry: Registry, save: EditSave) -> (EditSave, SaveState) {
    let cluster = &save.editor.target.cluster;
    let state = match registry.link(cluster) {
        None => SaveState::NotSent(CoreError::invalid(format!("{cluster} is not connected"))),
        Some(link) => send(&link.gateway, &save).await,
    };
    (save, state)
}

async fn send(gateway: &Gateway, save: &EditSave) -> SaveState {
    let target = &save.editor.target;
    let Some(version) = save.base.version.as_deref() else {
        return SaveState::NotSent(CoreError::invalid("the read has no resource version"));
    };
    let body = match save::body(&save.text, target.kind, version) {
        Ok(body) => body,
        Err(error) => return SaveState::Failed(error),
    };
    let check = save.stage == Stage::Check;
    let object = (target.kind, &target.key, &*target.uid);
    match save::replace(gateway, object, &body, check).await {
        Ok(text) if check => SaveState::Checked(Arc::new(text)),
        Ok(text) => SaveState::Saved(Arc::new(text)),
        Err(WriteError { fate: Fate::Refused, error }) if error.status() == Some(409) => {
            match remote(gateway, save).await {
                Ok(remote) => SaveState::Conflict(remote),
                Err(read) => {
                    SaveState::Failed(read.about("the object changed since the read, and its new read failed"))
                }
            }
        }
        Err(WriteError { fate: Fate::Refused, error }) => SaveState::Failed(error),
        Err(WriteError { fate: Fate::NotSent, error }) => SaveState::NotSent(error),
        // A dry run stores nothing, so its result is known.
        Err(WriteError { fate: Fate::Unknown, error }) if check => SaveState::Failed(error),
        Err(WriteError { fate: Fate::Unknown, error }) => SaveState::Unknown(error, remote(gateway, save).await.ok()),
    }
}

/// Reads the object again and merges the text of the save with it. The merge of 8,000 lines
/// can take milliseconds, so it runs on a thread for work that blocks.
async fn remote(gateway: &Gateway, save: &EditSave) -> Result<Arc<Remote>, CoreError> {
    let target = &save.editor.target;
    let read = Arc::new(read_for_edit(gateway, target.kind, &target.key, &target.uid).await?);
    let (base, text, now) = (Arc::clone(&save.base), Arc::clone(&save.text), Arc::clone(&read));
    let merge = tokio::task::spawn_blocking(move || merge::merge(&base.text, &text, &now.text))
        .await
        .map_err(|_| CoreError::invalid("the merge stopped"))?;
    Ok(Arc::new(Remote { read, merge }))
}
