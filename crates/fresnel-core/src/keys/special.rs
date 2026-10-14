//! The special views of some types of a Secret (plan section 12.2): the certificates of a
//! TLS Secret and the registries of an image pull Secret.

use super::pull::{self, Format, Registry};
use super::secret_type::SecretType;
use super::tls::{self, Certificate};

/// The special view of a Secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Special {
    /// The type has no special view.
    None,
    /// `kubernetes.io/tls`: each certificate of `tls.crt`, or why a block is none.
    Certificates(Vec<Result<Certificate, String>>),
    /// An image pull Secret: each registry with its user, or why the JSON does not read.
    Registries(Result<Vec<Registry>, String>),
}

/// The special view of a Secret of this type. `value` gives the bytes of a key.
pub(super) fn of<'a>(kind: &SecretType, value: impl Fn(&str) -> Option<&'a [u8]>) -> Special {
    let bytes = |key: &str| value(key).unwrap_or_default();
    match kind {
        SecretType::Tls => Special::Certificates(tls::certificates(bytes("tls.crt"))),
        SecretType::DockerConfigJson => {
            Special::Registries(pull::registries(bytes(".dockerconfigjson"), Format::ConfigJson))
        }
        SecretType::DockerCfg => Special::Registries(pull::registries(bytes(".dockercfg"), Format::Cfg)),
        _ => Special::None,
    }
}

/// Why the server or a client will not use a special value: a TLS certificate that does
/// not read, or a registry JSON that does not read. `None` for an absent or empty value:
/// the hint of the absent key tells that.
pub(super) fn problem<'a>(kind: &SecretType, value: impl Fn(&str) -> Option<&'a [u8]>) -> Option<String> {
    let key = match kind {
        SecretType::Tls => "tls.crt",
        SecretType::DockerConfigJson => ".dockerconfigjson",
        SecretType::DockerCfg => ".dockercfg",
        _ => return None,
    };
    value(key).filter(|bytes| !bytes.is_empty())?;
    let reason = match of(kind, value) {
        Special::Certificates(certificates) => certificates.into_iter().find_map(Result::err),
        Special::Registries(registries) => registries.err(),
        Special::None => None,
    };
    reason.map(|reason| format!("`{key}`: {reason}"))
}
