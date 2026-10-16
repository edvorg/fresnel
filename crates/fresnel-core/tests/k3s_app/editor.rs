//! The save of an editor through `Command::ShowEditors` and `Command::SaveEditor`, as the
//! editor sends them (plan section 12.9 and the acceptance of M11): a dry run, then the
//! replace with the version of the read; a hint never refuses a save; a conflict gives a
//! merge or the three versions; a Secret shows decoded and saves encoded. kubectl reads the
//! result in cluster b, and an assertion names keys only, never a value of a Secret.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_core::app::{Command, CoreView, EditRequest, EditSave, EditState, SaveState, Stage};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::EditText;
use fresnel_core::manifest::check::{self, Identity, Level};
use fresnel_core::manifest::merge::{Conflict, Side};
use fresnel_core::ops::Target;
use fresnel_core::store::ObjectKey;

use crate::bed::Bed;
use crate::scene::{APP, field, kubectl};

/// The editor of an object of the namespace `app` of cluster b, with its UID from kubectl.
async fn editor(bed: &Bed, kind: Kind, name: &str, number: u64) -> EditRequest {
    let object = format!("{}/{name}", kind.info().plural);
    let uid = field(&bed.b, &object, "{.metadata.uid}").await;
    let target = Target { cluster: ClusterId::new("b"), kind, key: ObjectKey::new(APP, name), uid: uid.into() };
    EditRequest { number, target }
}

/// Shows this editor and waits for its read.
async fn read(bed: &Bed, editor: &EditRequest) -> Arc<EditText> {
    bed.core.send(Command::ShowEditors(vec![editor.clone()]));
    let done = |view: &CoreView| {
        view.edits.iter().any(|read| read.request == *editor && matches!(read.state, EditState::Done(_)))
    };
    let view = bed.until("the read of the editor", done).await;
    let read = view.edits.iter().find(|read| read.request == *editor).map(|read| read.state.clone());
    let Some(EditState::Done(text)) = read else { panic!("the text of the editor") };
    text
}

/// Sends a save and waits for its end.
async fn save(bed: &Bed, save: EditSave) -> SaveState {
    bed.core.send(Command::SaveEditor(save.clone()));
    let ended = |view: &CoreView| {
        let read = view.edits.iter().find(|read| read.request == save.editor);
        let view = read.and_then(|read| read.save.as_ref()).filter(|view| view.save.number == save.number);
        view.is_some_and(|view| view.state != SaveState::Pending)
    };
    let view = bed.until("the end of the save", ended).await;
    let read = view.edits.iter().find(|read| read.request == save.editor).expect("the editor");
    read.save.as_ref().expect("the save").state.clone()
}

/// The dry run and then the replace of a text, as the editor sends them after its dialog.
/// Gives the answer of the replace: the new read of the editor.
async fn check_and_send(
    bed: &Bed,
    editor: &EditRequest,
    (number, base): (u64, &Arc<EditText>),
    text: &str,
) -> Arc<EditText> {
    let mut edit =
        EditSave { editor: editor.clone(), number, base: Arc::clone(base), text: text.into(), stage: Stage::Check };
    let SaveState::Checked(after) = save(bed, edit.clone()).await else { panic!("a dry run that passes") };
    assert!(after.text.contains("resourceVersion"), "the object as the server would store it");
    edit.number += 1;
    edit.stage = Stage::Send;
    match save(bed, edit).await {
        SaveState::Saved(answer) => answer,
        other => panic!("a save that the server stored: {other}"),
    }
}

/// A plain octal number gets a hint, and the save sends it as the parser and kubectl read
/// it: valid YAML is never refused by a hint.
pub(crate) async fn a_hint_never_refuses_a_save(bed: &Bed) {
    let web = editor(bed, Kind::Deployment, "web", 101).await;
    let base = read(bed, &web).await;
    let text = base.text.replace("progressDeadlineSeconds: 600", "progressDeadlineSeconds: 0700");
    assert_ne!(text, *base.text, "the read has the default deadline");
    let identity = Identity { kind: Kind::Deployment, key: web.target.key.clone() };
    let marks = check::check(&text, &identity).marks;
    assert!(
        marks.iter().any(|mark| mark.level == Level::Hint && mark.text.starts_with("0700 is the octal number 448"))
    );
    assert!(marks.iter().all(|mark| mark.level == Level::Hint), "a hint and no error");
    let answer = check_and_send(bed, &web, (1, &base), &text).await;
    assert!(answer.version != base.version, "a new version");
    assert_eq!(field(&bed.b, "deployment/web", "{.spec.progressDeadlineSeconds}").await, "448");
}

/// The text with the line of the key `key` of `data` set to this value, in double quotes.
fn set(text: &str, key: &str, value: &str) -> String {
    let start = format!("  {key}: ");
    let lines =
        text.lines().map(|line| if line.starts_with(&start) { format!("{start}\"{value}\"") } else { line.to_owned() });
    lines.map(|line| format!("{line}\n")).collect()
}

/// The line of the key `key` of `data` in a text, with its line feed.
fn line(text: &str, key: &str) -> String {
    let start = format!("  {key}: ");
    text.lines().find(|line| line.starts_with(&start)).map(|line| format!("{line}\n")).expect("the key in the text")
}

/// Changes of other lines merge after the conflict of the dry run, and the editor asks
/// again; a change of the same line by the server shows the three versions, and the side of
/// the user goes.
pub(crate) async fn a_conflict_gives_a_merge_or_a_diff_of_three_versions(bed: &Bed) {
    kubectl(
        &bed.b,
        &["create", "configmap", "settings", "--namespace", APP, "--from-literal=a=1", "--from-literal=b=2"],
    )
    .await;
    let settings = editor(bed, Kind::ConfigMap, "settings", 102).await;
    let base = read(bed, &settings).await;
    let patch = |key: &str, value: &str| format!(r#"{{"data":{{"{key}":"{value}"}}}}"#);
    kubectl(&bed.b, &["patch", "configmap", "settings", "--namespace", APP, "--patch", &patch("b", "20")]).await;
    let mine = set(&base.text, "a", "10");
    let edit = EditSave {
        editor: settings.clone(),
        number: 1,
        base: Arc::clone(&base),
        text: mine.into(),
        stage: Stage::Check,
    };
    let SaveState::Conflict(remote) = save(bed, edit).await else { panic!("a conflict of the dry run") };
    assert!(remote.merge.is_clean(), "changes of other lines: {:?}", remote.merge);
    let merged = remote.merge.text(|_| Side::Ours);
    assert_eq!((line(&merged, "a"), line(&merged, "b")), ("  a: \"10\"\n".to_owned(), line(&remote.read.text, "b")));
    let version = remote.read.version.clone().expect("a version");
    assert!(remote.read.version != base.version && merged.contains(&version), "the version of the new read");
    let saved = check_and_send(bed, &settings, (2, &remote.read), &merged).await;
    assert_eq!(field(&bed.b, "configmap/settings", "{.data.a} {.data.b}").await, "10 20");

    kubectl(&bed.b, &["patch", "configmap", "settings", "--namespace", APP, "--patch", &patch("a", "11")]).await;
    let mine = set(&saved.text, "a", "12");
    let edit = EditSave {
        editor: settings.clone(),
        number: 4,
        base: Arc::clone(&saved),
        text: mine.into(),
        stage: Stage::Check,
    };
    let SaveState::Conflict(remote) = save(bed, edit).await else { panic!("a conflict of the dry run") };
    let theirs = line(&remote.read.text, "a");
    assert!(theirs.contains("11"), "the line of the server: {theirs}");
    let three = Conflict { base: line(&saved.text, "a"), ours: "  a: \"12\"\n".into(), theirs };
    assert_eq!(remote.merge.conflicts().collect::<Vec<_>>(), [&three], "the three versions of the line");
    check_and_send(bed, &settings, (5, &remote.read), &remote.merge.text(|_| Side::Ours)).await;
    assert_eq!(field(&bed.b, "configmap/settings", "{.data.a} {.data.b}").await, "12 20");
}

/// The text of a Secret has each text value decoded and a binary value with `!!binary`; a
/// save encodes the changed value, and each other value stays byte for byte.
pub(crate) async fn a_secret_shows_decoded_and_saves_encoded(bed: &Bed) {
    let secret = ["create", "secret", "generic", "vault", "--namespace", APP];
    kubectl(&bed.b, &[&secret[..], &["--from-literal=password=hunter2", "--from-literal=note=a\nb\n"]].concat()).await;
    let binary = STANDARD.encode([0x89, b'P', b'N', b'G', 0xff, 0x00]);
    let logo = format!(r#"{{"data":{{"logo":"{binary}"}}}}"#);
    kubectl(&bed.b, &["patch", "secret", "vault", "--namespace", APP, "--patch", &logo]).await;
    let vault = editor(bed, Kind::Secret, "vault", 103).await;
    let base = read(bed, &vault).await;
    assert!(base.text.contains("  password: hunter2\n"), "the value shows decoded");
    assert!(base.text.contains(&format!("  logo: !!binary {binary}\n")), "a binary value stays base64 with its tag");
    let before = field(&bed.b, "secret/vault", "{.data.note} {.data.logo}").await;
    check_and_send(bed, &vault, (1, &base), &base.text.replace("password: hunter2", "password: hunter3")).await;
    assert!(field(&bed.b, "secret/vault", "{.data.password}").await == STANDARD.encode("hunter3"), "encoded again");
    assert!(field(&bed.b, "secret/vault", "{.data.note} {.data.logo}").await == before, "each other value stays");
    bed.core.send(Command::ShowEditors(Vec::new()));
    bed.until("the editors close", |view| view.edits.is_empty()).await;
}
