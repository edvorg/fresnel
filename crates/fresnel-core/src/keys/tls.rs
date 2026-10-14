//! The certificates of a TLS Secret (research 08 section 8.4): for each certificate of the
//! chain in `tls.crt`, its subject, issuer, validity, names, serial number and key.
//!
//! A text of this module never holds a byte of the key: `tls.key` is not read here.

use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use x509_parser::certificate::X509Certificate;
use x509_parser::extensions::GeneralName;
use x509_parser::oid_registry::{OID_SIG_ED448, OID_SIG_ED25519};
use x509_parser::pem::Pem;
use x509_parser::public_key::PublicKey;

/// The most blocks that a read takes. A chain has a few; the limit holds the work of a
/// value that is not a chain, and of a reader that would give an error at one place.
const MAX_BLOCKS: usize = 64;

/// The facts of one certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    /// The subject, as `CN=web, O=shop`.
    pub subject: String,
    /// The issuer, in the same form. A self-signed certificate has its subject here.
    pub issuer: String,
    /// The start of the validity.
    pub not_before: SystemTime,
    /// The end of the validity.
    pub not_after: SystemTime,
    /// The subject alternative names, as `DNS:web.shop`, `IP:10.0.0.1`, `email:a@b` or
    /// `URI:https://web`: the names that a client accepts.
    pub names: Vec<String>,
    /// The serial number, as hex bytes with colons.
    pub serial: String,
    /// The type and the size of the public key, as `RSA 2048 bits`.
    pub key: String,
    /// True for the certificate of a certificate authority.
    pub is_ca: bool,
}

impl Certificate {
    /// The whole days from `now` to the end of the validity. Negative after the end: the
    /// day of the end counts as day -1.
    pub fn days_left(&self, now: SystemTime) -> i64 {
        let days = |time: Duration| i64::try_from(time.as_secs() / 86_400).unwrap_or(i64::MAX);
        match self.not_after.duration_since(now) {
            Ok(left) => days(left),
            Err(past) => -days(past.duration() + Duration::from_secs(86_399)),
        }
    }
}

/// Each certificate of a chain in PEM form, in its sequence. A block that is no certificate
/// gives its reason in its place. Bytes with no PEM block give one reason.
pub(super) fn certificates(pem: &[u8]) -> Vec<Result<Certificate, String>> {
    let blocks = Pem::iter_from_buffer(pem).take(MAX_BLOCKS).enumerate();
    let mut found: Vec<_> = blocks
        .map(|(index, block)| {
            let number = index + 1;
            // A bad block does not end the read: a block after it can be good.
            match block {
                Err(_) => Err(format!("block {number} is not a PEM block")),
                Ok(block) if block.label != "CERTIFICATE" => {
                    Err(format!("block {number} is a {}, not a certificate", block.label))
                }
                Ok(block) => match block.parse_x509() {
                    Ok(certificate) => facts(&certificate).ok_or_else(|| format!("block {number} has an invalid time")),
                    Err(_) => Err(format!("block {number} is not an X.509 certificate")),
                },
            }
        })
        .collect();
    if found.is_empty() {
        found.push(Err("the value has no PEM certificate".to_owned()));
    }
    found
}

/// The facts of one certificate. `None` for a time before 1970 or past the range of the
/// clock of the system.
fn facts(certificate: &X509Certificate<'_>) -> Option<Certificate> {
    let validity = certificate.validity();
    Some(Certificate {
        subject: certificate.subject().to_string(),
        issuer: certificate.issuer().to_string(),
        not_before: time(validity.not_before.timestamp())?,
        not_after: time(validity.not_after.timestamp())?,
        names: names(certificate),
        serial: certificate.raw_serial_as_string(),
        key: key(certificate),
        is_ca: certificate.is_ca(),
    })
}

fn time(seconds: i64) -> Option<SystemTime> {
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(u64::try_from(seconds).ok()?))
}

/// The subject alternative names. A certificate with a broken extension shows none: its
/// subject and its dates are still of use.
fn names(certificate: &X509Certificate<'_>) -> Vec<String> {
    let Ok(Some(extension)) = certificate.subject_alternative_name() else {
        return Vec::new();
    };
    let name = |general: &GeneralName<'_>| match general {
        GeneralName::DNSName(name) => Some(format!("DNS:{name}")),
        GeneralName::RFC822Name(mail) => Some(format!("email:{mail}")),
        GeneralName::URI(uri) => Some(format!("URI:{uri}")),
        GeneralName::IPAddress(bytes) => address(bytes).map(|ip| format!("IP:{ip}")),
        _ => None,
    };
    extension.value.general_names.iter().filter_map(name).collect()
}

fn address(bytes: &[u8]) -> Option<IpAddr> {
    match bytes.len() {
        4 => <[u8; 4]>::try_from(bytes).ok().map(IpAddr::from),
        16 => <[u8; 16]>::try_from(bytes).ok().map(IpAddr::from),
        _ => None,
    }
}

/// The type and the size of the public key.
fn key(certificate: &X509Certificate<'_>) -> String {
    let info = certificate.public_key();
    match info.parsed() {
        Ok(PublicKey::RSA(rsa)) => format!("RSA {} bits", rsa.key_size()),
        Ok(PublicKey::EC(point)) => format!("EC {} bits", point.key_size()),
        Ok(PublicKey::DSA(_)) => "DSA".to_owned(),
        _ if info.algorithm.algorithm == OID_SIG_ED25519 => "Ed25519".to_owned(),
        _ if info.algorithm.algorithm == OID_SIG_ED448 => "Ed448".to_owned(),
        _ => format!("algorithm {}", info.algorithm.algorithm),
    }
}

#[cfg(test)]
mod tests;
