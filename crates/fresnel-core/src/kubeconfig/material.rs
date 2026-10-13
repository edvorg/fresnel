//! Bytes that a kubeconfig gives inline or as the path of a file.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::error::CoreError;

/// A certificate or a key of a kubeconfig entry. It has no `Debug`: it can be a key.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Material {
    /// The bytes of a `-data` field, decoded.
    Inline(Vec<u8>),
    /// A file. The app reads it at each connect, so a new file needs no new load.
    File(PathBuf),
}

impl Material {
    /// The material of one pair of fields. The `-data` field wins, as in `kubectl`. A
    /// relative path starts from the folder of the kubeconfig file.
    pub(super) fn of(
        data: Option<&str>,
        file: Option<&str>,
        folder: &Path,
        what: &str,
    ) -> Result<Option<Self>, CoreError> {
        if let Some(data) = data.filter(|data| !data.is_empty()) {
            let compact: String = data.split_ascii_whitespace().collect();
            let decoded = STANDARD.decode(compact);
            return decoded.map(|bytes| Some(Self::Inline(bytes))).map_err(|_| not_base64(what));
        }
        Ok(file.filter(|file| !file.is_empty()).map(|file| Self::File(folder.join(file))))
    }

    /// The bytes. This call reads a file, so it can block for a short time.
    pub(crate) fn read(&self, what: &str) -> Result<Vec<u8>, CoreError> {
        match self {
            Self::Inline(bytes) => Ok(bytes.clone()),
            Self::File(path) => std::fs::read(path)
                .map_err(|error| CoreError::invalid(format!("cannot read the {what} {}: {error}", path.display()))),
        }
    }
}

fn not_base64(what: &str) -> CoreError {
    CoreError::invalid(format!("the {what} data of the kubeconfig is not base64"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_field_wins_and_a_relative_path_starts_from_the_folder() {
        let folder = Path::new("/home/user/.kube");
        let of = |data, file| Material::of(data, file, folder, "client key");
        assert!(of(Some("YWJj"), Some("key.pem")).expect("base64") == Some(Material::Inline(b"abc".to_vec())));
        assert!(of(Some("YW\n  Jj\n"), None).expect("base64 in lines") == Some(Material::Inline(b"abc".to_vec())));
        let file = |path: &str| Some(Material::File(PathBuf::from(path)));
        assert!(of(None, Some("certs/key.pem")).expect("a path") == file("/home/user/.kube/certs/key.pem"));
        assert!(of(Some(""), Some("/etc/key.pem")).expect("a path") == file("/etc/key.pem"));
        assert!(of(None, None).expect("nothing").is_none());

        let error = of(Some("not base64!"), None).err().expect("bad data");
        assert_eq!(error.to_string(), "the client key data of the kubeconfig is not base64");
    }

    #[test]
    fn a_file_that_is_absent_gives_an_error_with_its_path() {
        let error = Material::File(PathBuf::from("/no/such/ca.pem")).read("certificate authority");
        let text = error.expect_err("no such file").to_string();
        assert!(text.starts_with("cannot read the certificate authority /no/such/ca.pem: "), "{text}");
    }
}
