//! From a kubeconfig entry to what the connection layer needs for one cluster.

use std::sync::Arc;

use fresnel_net::env::ProcessEnv;
use fresnel_net::{ClusterSpec, Endpoint, NetSettings, WallClock};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore};

use super::ClusterId;
use crate::error::CoreError;
use crate::kubeconfig::Access;

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
        tls: Arc::new(tls_settings(&authority, access)?),
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
fn tls_settings(authority: &[u8], access: &Access) -> Result<ClientConfig, CoreError> {
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
    let Some(identity) = &access.identity else {
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
