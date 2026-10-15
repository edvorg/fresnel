//! The files that the user saves into the folder of downloads: a log of a view and the
//! manifest of an object. Each one is a new file, never one that exists, and its name tells
//! the cluster, the namespace, the kind and the name of its object.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use k8s_openapi::jiff::Timestamp;

use crate::cluster::ClusterId;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// The most characters of the cluster and of the object in the name of a file. A file system
/// takes a name of 255 bytes at most, and a context or a pod can have a name of 253: the
/// longest file name has 60 + 63 + 32 + 60 + 40 bytes.
const NAME_PART: usize = 60;
/// The most characters of a namespace: its limit in Kubernetes.
const NAMESPACE: usize = 63;
/// The most names that a save tries when files of its name exist.
const NAMES: u32 = 100;

/// The start of the name of a file of an object: `ams-prod_shop_pod-web-1`, each part cut to
/// its limit. Each character that some file system or shell takes in a special way is `_`.
pub(crate) fn stem(cluster: &ClusterId, kind: Kind, key: &ObjectKey) -> String {
    let kind = kind.info().name.to_ascii_lowercase();
    let (cluster, namespace) = (safe(cluster.as_str(), NAME_PART), safe(&key.namespace, NAMESPACE));
    format!("{cluster}_{namespace}_{kind}-{}", safe(&key.name, NAME_PART))
}

/// The time of a file name in UTC, for example `20270115-080000`.
pub(crate) fn time(at: SystemTime) -> String {
    Timestamp::try_from(at).map_or_else(|_| "no-time".to_owned(), |at| at.strftime("%Y%m%d-%H%M%S").to_string())
}

/// The part of a file name for an attempt: none for the first one, `-2` for the second one.
pub(crate) fn number(attempt: u32) -> String {
    if attempt == 0 { String::new() } else { format!("-{}", attempt + 1) }
}

/// Makes the folder and a new file in it. `name` gives the name of each attempt, from 0: a
/// later attempt comes when a file of the name exists.
pub(crate) fn create(folder: &Path, name: impl Fn(u32) -> String) -> io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(folder)?;
    let mut attempt = 0;
    loop {
        let path = folder.join(name(attempt));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt + 1 < NAMES => attempt += 1,
            Err(error) => return Err(error),
        }
    }
}

/// The first `most` characters of a text, each one that is not a letter, a digit, `.`, `-` or
/// `_` as `_`.
fn safe(text: &str, most: usize) -> String {
    let safe = |c: char| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' };
    text.chars().take(most).map(safe).collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_stem_has_each_part_with_no_special_character_and_a_new_file_never_replaces_one() {
        let stem = stem(&ClusterId::new("ams prod"), Kind::ConfigMap, &ObjectKey::new("shop", "web/1"));
        assert_eq!(stem, "ams_prod_shop_configmap-web_1");
        assert_eq!(time(SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)), "20270115-080000");
        assert_eq!((number(0), number(2)), (String::new(), "-3".to_owned()));

        let folder = std::env::temp_dir().join(format!("fresnel-downloads-{:08x}", fastrand::u32(..)));
        let name = |attempt| format!("a{}.yaml", number(attempt));
        let (_, first) = create(&folder, name).expect("a first file");
        let (_, second) = create(&folder, name).expect("a second file");
        assert_eq!((first.file_name(), second.file_name()), (Some("a.yaml".as_ref()), Some("a-2.yaml".as_ref())));
        std::fs::remove_dir_all(&folder).expect("the folder of the test goes");
    }
}
