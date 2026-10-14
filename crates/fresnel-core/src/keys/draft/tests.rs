use std::collections::BTreeMap;

use super::*;
use crate::cluster::ClusterId;
use crate::keys::object::Entry;
use crate::keys::secret_type::SecretType;
use crate::store::ObjectKey;

/// A read of an object of this kind with these keys, each as name, map and bytes.
fn read(kind: KeyKind, version: &str, keys: &[(&str, Place, &[u8])]) -> Arc<KeyObject> {
    let entries: BTreeMap<String, Entry> = keys
        .iter()
        .map(|(key, place, bytes)| ((*key).to_owned(), Entry { place: *place, value: Value::new(bytes.to_vec()) }))
        .collect();
    let target = Target { cluster: ClusterId::new("ams"), kind, key: ObjectKey::new("shop", "db"), uid: "u-db".into() };
    let secret_type = (kind == KeyKind::Secret).then_some(SecretType::Opaque);
    Arc::new(KeyObject { target, version: version.into(), immutable: false, secret_type, entries })
}

fn secret(version: &str, keys: &[(&str, &[u8])]) -> Arc<KeyObject> {
    let keys: Vec<(&str, Place, &[u8])> = keys.iter().map(|(key, bytes)| (*key, Place::Data, *bytes)).collect();
    read(KeyKind::Secret, version, &keys)
}

fn states(draft: &Draft) -> Vec<(&str, KeyState, &[u8])> {
    draft.keys().into_iter().map(|key| (key.name, key.state, key.value.bytes())).collect()
}

fn write(key: &str, new: Option<(Place, &[u8])>, old: Option<Place>) -> KeyWrite {
    KeyWrite { key: key.to_owned(), new: new.map(|(place, bytes)| (place, Value::new(bytes.to_vec()))), old }
}

#[test]
fn a_change_add_and_remove_show_in_the_draft_and_the_save_has_only_the_changed_keys() {
    let mut draft = Draft::new(secret("41", &[("a", b"1"), ("b", b"\x00\xff"), ("c", b"3")]));
    assert!(!draft.is_changed());
    assert_eq!(draft.save(), Err(Refusal::NothingToSave));

    draft.set("a", b"one".to_vec()).expect("a change");
    draft.add("d", b"4".to_vec()).expect("a new key");
    draft.remove("c").expect("a removal");
    assert_eq!(
        states(&draft),
        [
            ("a", KeyState::Changed, &b"one"[..]),
            ("b", KeyState::Same, &b"\x00\xff"[..]),
            ("c", KeyState::Removed, &b"3"[..]),
            ("d", KeyState::Added, &b"4"[..]),
        ]
    );
    assert_eq!((draft.value("c"), draft.value("d").map(Value::bytes)), (None, Some(&b"4"[..])));

    let save = draft.save().expect("a save");
    assert_eq!((&save.target, &*save.version), (&draft.base().target, "41"));
    assert_eq!(
        save.writes,
        [
            write("a", Some((Place::Data, b"one")), Some(Place::Data)),
            write("c", None, Some(Place::Data)),
            write("d", Some((Place::Data, b"4")), None),
        ]
    );
}

#[test]
fn a_change_back_to_the_bytes_of_the_read_is_no_change() {
    let mut draft = Draft::new(secret("41", &[("a", b"1"), ("b", b"2")]));
    draft.set("a", b"x".to_vec()).expect("a change");
    draft.set("a", b"1".to_vec()).expect("a change back");
    assert!(!draft.is_changed(), "the bytes of the read again");

    // A text editor that adds a line feed changes the value: the save sends it.
    draft.set("b", b"2\n".to_vec()).expect("a change");
    assert_eq!(draft.changes().keys().collect::<Vec<_>>(), ["b"]);

    draft.remove("a").expect("a removal");
    draft.add("a", b"1".to_vec()).expect("the key comes back");
    assert_eq!(draft.changes().keys().collect::<Vec<_>>(), ["b"], "the same bytes come back: no change");
    draft.add("e", b"5".to_vec()).expect("a new key");
    draft.remove("e").expect("a new key goes");
    draft.undo("b");
    assert!(!draft.is_changed());
}

#[test]
fn an_edit_of_a_key_that_is_not_there_or_with_a_bad_name_is_refused() {
    let mut draft = Draft::new(secret("41", &[("a", b"1")]));
    assert_eq!(draft.set("z", b"1".to_vec()), Err(Refusal::NoSuchKey("z".to_owned())));
    assert_eq!(draft.remove("z"), Err(Refusal::NoSuchKey("z".to_owned())));
    assert_eq!(draft.add("a", b"2".to_vec()), Err(Refusal::KeyExists("a".to_owned())));
    let bad = draft.add("a/b", b"2".to_vec());
    assert_eq!(bad, Err(Refusal::BadName("`a/b`: a key has only letters, digits, `-`, `_` and `.`".to_owned())));
    draft.remove("a").expect("a removal");
    assert_eq!(draft.set("a", b"2".to_vec()), Err(Refusal::NoSuchKey("a".to_owned())), "a removed key");
    assert_eq!(draft.remove("a"), Err(Refusal::NoSuchKey("a".to_owned())));
    assert_eq!(Refusal::KeyExists("a".to_owned()).to_string(), "the key `a` exists");
}

#[test]
fn an_immutable_object_refuses_each_edit_and_each_save() {
    let mut base = KeyObject::clone(&secret("41", &[("a", b"1")]));
    base.immutable = true;
    let mut draft = Draft::new(Arc::new(base));
    assert_eq!(draft.set("a", b"2".to_vec()), Err(Refusal::Immutable));
    assert_eq!(draft.add("b", b"2".to_vec()), Err(Refusal::Immutable));
    assert_eq!(draft.remove("a"), Err(Refusal::Immutable));
    assert_eq!(draft.save(), Err(Refusal::Immutable));
    assert!(!draft.is_changed());
    assert_eq!(Refusal::Immutable.to_string(), "the object is immutable: only a delete and a new create change it");
}

#[test]
fn a_config_map_key_stays_in_its_map_while_its_value_fits_there() {
    let base = read(
        KeyKind::ConfigMap,
        "7",
        &[
            ("conf", Place::Data, b"port: 80\n"),
            ("logo", Place::BinaryData, b"\x89PNG"),
            ("note", Place::BinaryData, b"plain text"),
        ],
    );
    let mut draft = Draft::new(base);
    draft.set("conf", b"port: 81\n".to_vec()).expect("text in data");
    draft.set("note", b"other text".to_vec()).expect("text that stays in binaryData");
    draft.set("logo", b"\x89PNG2".to_vec()).expect("bytes in binaryData");
    draft.add("new.txt", b"hello".to_vec()).expect("a new text");
    draft.add("new.bin", b"\xff\x00".to_vec()).expect("new bytes");
    let save = draft.save().expect("a save");
    assert_eq!(
        save.writes,
        [
            write("conf", Some((Place::Data, b"port: 81\n")), Some(Place::Data)),
            write("logo", Some((Place::BinaryData, b"\x89PNG2")), Some(Place::BinaryData)),
            write("new.bin", Some((Place::BinaryData, b"\xff\x00")), None),
            write("new.txt", Some((Place::Data, b"hello")), None),
            write("note", Some((Place::BinaryData, b"other text")), Some(Place::BinaryData)),
        ]
    );

    // A value in `data` that is not UTF-8 any more moves to `binaryData`.
    let mut draft = Draft::new(read(KeyKind::ConfigMap, "7", &[("conf", Place::Data, b"x")]));
    draft.set("conf", b"\xfe".to_vec()).expect("bytes");
    let moved = draft.save().expect("a save");
    assert_eq!(moved.writes, [write("conf", Some((Place::BinaryData, b"\xfe")), Some(Place::Data))]);
}

#[test]
fn a_new_read_keeps_the_changes_drops_the_ones_that_the_server_has_and_tells_each_overlap() {
    let mut draft = Draft::new(secret("41", &[("a", b"1"), ("b", b"2"), ("c", b"3"), ("d", b"4"), ("e", b"5")]));
    draft.set("a", b"mine".to_vec()).expect("a change that the server makes too");
    draft.set("b", b"same".to_vec()).expect("a change that the server has already");
    draft.set("c", b"only mine".to_vec()).expect("a change that the server does not touch");
    draft.remove("d").expect("a removal of a key that the server changes");
    draft.add("f", b"new".to_vec()).expect("a key that the server adds too");
    let server =
        secret("50", &[("a", b"theirs"), ("b", b"same"), ("c", b"3"), ("d", b"four"), ("e", b"five"), ("f", b"f")]);

    let conflicts = draft.rebase(Arc::clone(&server)).expect("the same object");
    let value = |bytes: &[u8]| Some(Value::new(bytes.to_vec()));
    assert_eq!(
        conflicts,
        [
            Conflict { key: "a".to_owned(), base: value(b"1"), mine: value(b"mine"), server: value(b"theirs") },
            Conflict { key: "d".to_owned(), base: value(b"4"), mine: None, server: value(b"four") },
            Conflict { key: "f".to_owned(), base: None, mine: value(b"new"), server: value(b"f") },
        ]
    );
    assert!(Arc::ptr_eq(draft.base(), &server), "the new read is the base");
    assert_eq!(draft.changes().keys().collect::<Vec<_>>(), ["a", "c", "d", "f"], "b is on the server already");
    let save = draft.save().expect("a save");
    assert_eq!(&*save.version, "50", "the save goes with the version of the new read");
    assert!(save.writes.iter().all(|write| write.key != "e"), "a key that only the server changed is not sent");

    draft.undo("a");
    assert_eq!(draft.value("a").map(Value::bytes), Some(&b"theirs"[..]), "an undo takes the value of the server");

    let mut other = Draft::new(secret("41", &[]));
    let mut moved = KeyObject::clone(&server);
    moved.target.uid = "u-new".into();
    assert_eq!(other.rebase(Arc::new(moved)), Err(Refusal::OtherObject), "a new object of the name");
}

#[test]
fn a_removal_that_the_server_made_too_is_no_change_and_no_conflict() {
    let mut draft = Draft::new(secret("41", &[("a", b"1"), ("b", b"2")]));
    draft.remove("a").expect("a removal");
    let conflicts = draft.rebase(secret("42", &[("b", b"2")])).expect("the same object");
    assert_eq!((conflicts, draft.is_changed()), (Vec::new(), false));
}
