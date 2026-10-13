//! The probe on real sockets: the steps of a connect that the `doctor` command measures.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use fresnel_net::probe::Probe;
use fresnel_net::{ConnectSettings, ErrorClass, Protocol};
use fresnel_testkit::{FakeApiServer, FaultProxy, ProxyMode};
use rcgen::CertifiedKey;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;

use crate::common::{spec, trust};

const MS: Duration = Duration::from_millis(1);
const SECOND: Duration = Duration::from_secs(1);

fn probe(address: SocketAddr, tls: Arc<ClientConfig>, settings: ConnectSettings) -> Probe {
    Probe::new(&spec("probe", address, tls).endpoint, settings).expect("a probe")
}

#[tokio::test]
async fn a_plain_connect_gives_the_time_of_each_step() {
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    // Each byte needs 30 ms in each direction. The kernel accepts a connection of the proxy at
    // once, so the round trip of the TCP handshake shows in the TLS step, before its own one.
    proxy.set(ProxyMode::Latency(30 * MS));
    let probe = probe(proxy.addr(), trust(&server), ConnectSettings::default());

    let lookup = probe.lookup().await.expect("an address needs no lookup");
    assert_eq!((lookup.time, lookup.addresses.as_slice()), (Duration::ZERO, &[proxy.addr()][..]));
    for protocol in [Protocol::Http2, Protocol::Http1] {
        let connect = probe.plain(proxy.addr(), protocol).await.expect("the server offers the two protocols");
        assert!(connect.tcp < 30 * MS && connect.tls >= 120 * MS, "{protocol:?}: {connect:?}");
        assert!(connect.tls < SECOND, "{protocol:?}: {connect:?}");
    }
    assert_eq!(server.counters().requests, 0, "a probe sends no request");
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}

#[tokio::test]
async fn the_lookup_of_a_name_gives_its_addresses_and_a_bad_name_gives_an_error() {
    let server = FakeApiServer::start().await.expect("a server");
    let mut spec = spec("names", server.addr(), trust(&server));
    spec.endpoint.host = "localhost".to_owned();
    let lookup = Probe::new(&spec.endpoint, ConnectSettings::default()).expect("a probe").lookup().await;
    let lookup = lookup.expect("each system knows localhost");
    assert!(lookup.addresses.contains(&server.addr()), "{lookup:?}");

    // The top-level domain `invalid` never has a name (RFC 6761).
    spec.endpoint.host = "fresnel-test.invalid".to_owned();
    let error = Probe::new(&spec.endpoint, ConnectSettings::default()).expect("a probe").lookup().await;
    let error = error.expect_err("no such name");
    assert!(error.to_string().starts_with("cannot resolve fresnel-test.invalid: "), "{error}");
}

/// Held connections: a plain attempt waits to its limit, and a hedged connect is fast.
#[tokio::test]
async fn a_held_connection_stops_a_plain_attempt_and_not_a_hedged_connect() {
    const SAMPLES: usize = 12;
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    proxy.set(ProxyMode::HoldNew { probability: 0.3, min: 5 * SECOND, max: 20 * SECOND });
    let settings = ConnectSettings { attempt_limit: 400 * MS, ..ConnectSettings::default() };
    let probe = probe(proxy.addr(), trust(&server), settings);

    let mut held = 0;
    for _ in 0..SAMPLES {
        match probe.plain(proxy.addr(), Protocol::Http2).await {
            Ok(connect) => assert!(connect.tcp + connect.tls < 400 * MS, "{connect:?}"),
            Err(error) => {
                assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Timeout, "no answer in 400ms"));
                held += 1;
            }
        }
    }
    assert_eq!(held, proxy.counters().held, "each held connection is one attempt with no answer");
    assert!(held >= 1, "the proxy held no connection: the fault was not present");

    let mut attempts = 0;
    for _ in 0..SAMPLES {
        let connect = probe.hedged(Protocol::Http2).await.expect("a connection");
        assert!(connect.time < SECOND, "{connect:?}");
        attempts += connect.attempts;
    }
    let status = probe.status();
    assert_eq!((status.connects, status.attempts), (SAMPLES as u64, attempts as u64));
    assert!(attempts > SAMPLES, "no connect started a second attempt");
    assert!(proxy.counters().held > held, "the proxy held no connection of a hedged connect");
    assert!(status.connect_time.expect("a typical time") < 150 * MS, "{status:?}");
    assert_eq!(status.hedge_delay, 150 * MS, "a near server gets the shortest delay");
}

/// A TLS server that offers HTTP/1.1 only, as an old proxy before an API server.
async fn serve_http1_only(listener: TcpListener, acceptor: TlsAcceptor) {
    let mut connections = JoinSet::new();
    loop {
        let (tcp, _peer) = listener.accept().await.expect("an accept");
        let acceptor = acceptor.clone();
        connections.spawn(async move {
            // The connection stays open until the client closes it.
            if let Ok(mut tls) = acceptor.accept(tcp).await {
                let mut rest = Vec::new();
                match tls.read_to_end(&mut rest).await {
                    Ok(_) | Err(_) => {}
                }
            }
        });
    }
}

#[tokio::test]
async fn a_server_with_no_http2_fails_the_connect_of_a_channel_with_that_reason() {
    let CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("a certificate");
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(vec![cert.der().clone()], key)
        .expect("a server identity");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let address = listener.local_addr().expect("an address");

    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(cert.der().to_vec())).expect("a certificate");
    let builder = ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions();
    let tls = Arc::new(builder.expect("versions").with_root_certificates(roots).with_no_client_auth());
    let probe = probe(address, tls, ConnectSettings::default());

    let checks = async {
        let upgrade = probe.plain(address, Protocol::Http1).await;
        assert!(upgrade.is_ok(), "{upgrade:?}");
        assert!(probe.hedged(Protocol::Http1).await.is_ok());

        // The TLS code of this server refuses the handshake. A server that ignores the
        // offer gets the same result from the check of the selected protocol.
        let error = probe.plain(address, Protocol::Http2).await.expect_err("the server has no HTTP/2");
        assert_eq!(error.to_string(), "the TLS handshake failed: received fatal alert: NoApplicationProtocol");
        let started = std::time::Instant::now();
        let error = probe.hedged(Protocol::Http2).await.expect_err("the server has no HTTP/2");
        assert_eq!(error.class(), ErrorClass::Network, "{error}");
        assert!(error.to_string().contains("attempt 2: received fatal alert: NoApplicationProtocol"), "{error}");
        assert!(started.elapsed() < SECOND, "a refusal needs no wait for a time limit: {:?}", started.elapsed());
    };
    tokio::select! {
        () = checks => {}
        () = serve_http1_only(listener, TlsAcceptor::from(Arc::new(config))) => {}
    }
}
