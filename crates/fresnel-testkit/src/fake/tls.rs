//! The TLS identity of the fake server: a new self-signed certificate for each start.
//!
//! The repository holds no key file. A test gives the certificate to its client as the one
//! trusted certificate.

use std::sync::Arc;

use rcgen::CertifiedKey;
use rustls::ServerConfig;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::TlsAcceptor;

use crate::KitError;

/// The names in the certificate. The server listens on 127.0.0.1.
const NAMES: [&str; 2] = ["127.0.0.1", "localhost"];

pub(super) struct Identity {
    pub(super) acceptor: TlsAcceptor,
    /// The certificate in DER form.
    pub(super) certificate: Vec<u8>,
}

/// Makes the identity. The server offers HTTP/2 and HTTP/1.1, as a kube-apiserver does.
pub(super) fn identity() -> Result<Identity, KitError> {
    let CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(NAMES.map(str::to_owned))
        .map_err(|error| KitError::because("cannot make the certificate of the fake server", error))?;
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|builder| builder.with_no_client_auth().with_single_cert(vec![cert.der().clone()], key))
        .map_err(|error| KitError::because("cannot make the TLS settings of the fake server", error))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Identity { acceptor: TlsAcceptor::from(Arc::new(config)), certificate: cert.der().to_vec() })
}
