//! From a kubeconfig entry to what the connection layer needs for one cluster.

use std::sync::Arc;

use fresnel_net::env::ProcessEnv;
use fresnel_net::{ClusterSpec, Endpoint, NetSettings, WallClock};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore};

use super::ClusterId;
use crate::error::CoreError;
use crate::kubeconfig::{Access, Identity};

/// Builds the spec of one cluster. The call reads each certificate file of the entry, so
/// it can block for a short time.
pub(super) fn cluster_spec(
    id: &ClusterId,
    access: &Access,
    env: &ProcessEnv,
    settings: NetSettings,
) -> Result<ClusterSpec, CoreError> {
    let authority = access.authority.read("certificate authority")?;
    let endpoint = Endpoint {
        host: access.server.host.clone(),
        port: access.server.port,
        base_path: access.server.base_path.clone(),
        tls_server_name: access.tls_server_name.clone(),
        tls: Arc::new(tls_settings(&authority, access.identity.as_ref())?),
    };
    Ok(ClusterSpec {
        name: id.to_string(),
        endpoint,
        plugin: access.plugin_for_connect(&authority),
        env: env.clone(),
        settings,
        clock: WallClock::system(),
    })
}

/// The TLS settings: the app trusts only the certificate authority of the kubeconfig.
fn tls_settings(authority: &[u8], identity: Option<&Identity>) -> Result<ClientConfig, CoreError> {
    let mut roots = RootCertStore::empty();
    for certificate in certificates(authority, "certificate authority")? {
        let added = roots.add(certificate);
        added.map_err(|error| CoreError::invalid(format!("a certificate of the certificate authority: {error}")))?;
    }
    if roots.is_empty() {
        return Err(CoreError::invalid("the certificate authority of the kubeconfig holds no certificate"));
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| CoreError::invalid(format!("the TLS settings: {error}")))?
        .with_root_certificates(roots);
    let Some(identity) = identity else {
        return Ok(builder.with_no_client_auth());
    };
    let chain = certificates(&identity.certificate.read("client certificate")?, "client certificate")?;
    let key = PrivateKeyDer::from_pem_slice(&identity.key.read("client key")?)
        .map_err(|_| CoreError::invalid("the client key of the kubeconfig is not a PEM key"))?;
    let with_identity = builder.with_client_auth_cert(chain, key);
    with_identity.map_err(|error| CoreError::invalid(format!("the client certificate of the kubeconfig: {error}")))
}

fn certificates(pem: &[u8], what: &str) -> Result<Vec<CertificateDer<'static>>, CoreError> {
    let read: Result<Vec<_>, _> = CertificateDer::pem_slice_iter(pem).collect();
    read.map_err(|_| CoreError::invalid(format!("the {what} of the kubeconfig is not a PEM certificate")))
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use rcgen::{CertifiedKey, generate_simple_self_signed};

    use super::*;
    use crate::kubeconfig::Material;

    fn pem(label: &str, der: &[u8]) -> Vec<u8> {
        format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", STANDARD.encode(der)).into_bytes()
    }

    /// A new certificate and its key, in PEM form.
    fn pair() -> (Vec<u8>, Vec<u8>) {
        let CertifiedKey { cert, signing_key } =
            generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("a certificate");
        (pem("CERTIFICATE", cert.der()), pem("PRIVATE KEY", &signing_key.serialize_der()))
    }

    fn identity(certificate: &[u8], key: &[u8]) -> Identity {
        Identity { certificate: Material::Inline(certificate.to_vec()), key: Material::Inline(key.to_vec()) }
    }

    #[test]
    fn the_tls_settings_carry_the_client_certificate_of_the_kubeconfig() {
        let (authority, _) = pair();
        let (certificate, key) = pair();
        let anonymous = tls_settings(&authority, None).expect("settings with no client certificate");
        assert!(!anonymous.client_auth_cert_resolver.has_certs());
        let known = tls_settings(&authority, Some(&identity(&certificate, &key))).expect("settings");
        assert!(known.client_auth_cert_resolver.has_certs());
    }

    #[test]
    fn a_bad_certificate_or_key_gives_its_reason() {
        let (authority, other_key) = pair();
        let (certificate, key) = pair();
        let error = |authority: &[u8], identity: Option<&Identity>| {
            tls_settings(authority, identity).expect_err("bad settings").to_string()
        };
        assert_eq!(error(b"ABC", None), "the certificate authority of the kubeconfig holds no certificate");
        assert_eq!(
            error(&authority, Some(&identity(&certificate, b"ABC"))),
            "the client key of the kubeconfig is not a PEM key"
        );
        let mismatch = error(&authority, Some(&identity(&certificate, &other_key)));
        assert!(mismatch.starts_with("the client certificate of the kubeconfig: "), "{mismatch}");
        let no_file = Identity { certificate: Material::File("/no/such/file.pem".into()), key: Material::Inline(key) };
        let text = error(&authority, Some(&no_file));
        assert!(text.starts_with("cannot read the client certificate /no/such/file.pem: "), "{text}");
    }
}
