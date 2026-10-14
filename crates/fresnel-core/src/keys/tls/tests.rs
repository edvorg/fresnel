use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, date_time_ymd};

use super::*;

fn pem(label: &str, der: &[u8]) -> String {
    format!("-----BEGIN {label}-----\n{}\n-----END {label}-----\n", STANDARD.encode(der))
}

/// Seconds after 1970 of a day at midnight UTC.
fn day(year: i32, month: u8, day: u8) -> SystemTime {
    let seconds = date_time_ymd(year, month, day).unix_timestamp();
    SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).expect("after 1970"))
}

/// A chain of two: a server certificate with names, and the authority that signed it.
fn chain() -> String {
    let mut authority = CertificateParams::new(Vec::new()).expect("parameters");
    authority.distinguished_name.push(DnType::CommonName, "shop CA");
    authority.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    authority.not_before = date_time_ymd(2026, 1, 1);
    authority.not_after = date_time_ymd(2036, 1, 1);
    let authority_key = KeyPair::generate().expect("a key");
    let authority_certificate = authority.self_signed(&authority_key).expect("the authority");

    let names = vec!["web.shop.example".to_owned(), "10.0.0.7".to_owned(), "::1".to_owned()];
    let mut server = CertificateParams::new(names).expect("parameters");
    server.distinguished_name.push(DnType::CommonName, "web");
    server.distinguished_name.push(DnType::OrganizationName, "shop");
    server.not_before = date_time_ymd(2026, 10, 1);
    server.not_after = date_time_ymd(2026, 12, 30);
    let server_key = KeyPair::generate().expect("a key");
    let issuer = Issuer::new(authority, authority_key);
    let server_certificate = server.signed_by(&server_key, &issuer).expect("the server certificate");
    pem("CERTIFICATE", server_certificate.der()) + &pem("CERTIFICATE", authority_certificate.der())
}

#[test]
fn each_certificate_of_a_chain_gives_its_subject_issuer_validity_and_names() {
    let found = certificates(chain().as_bytes());
    let [Ok(server), Ok(authority)] = found.as_slice() else {
        panic!("two certificates: {found:?}");
    };
    assert_eq!((server.subject.as_str(), server.issuer.as_str()), ("CN=web, O=shop", "CN=shop CA"));
    assert_eq!((server.not_before, server.not_after), (day(2026, 10, 1), day(2026, 12, 30)));
    assert_eq!(server.names, ["DNS:web.shop.example", "IP:10.0.0.7", "IP:::1"]);
    assert_eq!((server.key.as_str(), server.is_ca), ("EC 256 bits", false));
    assert!(server.serial.contains(':') && !server.serial.is_empty(), "{}", server.serial);

    assert_eq!((authority.subject.as_str(), authority.issuer.as_str()), ("CN=shop CA", "CN=shop CA"));
    assert_eq!((authority.names.len(), authority.is_ca), (0, true));
}

#[test]
fn the_days_left_count_whole_days_and_the_day_of_the_end_is_day_minus_one() {
    let found = certificates(chain().as_bytes());
    let Some(Ok(server)) = found.first() else { panic!("a certificate: {found:?}") };
    let hour = Duration::from_secs(3600);
    assert_eq!(server.days_left(day(2026, 10, 15)), 76);
    assert_eq!(server.days_left(day(2026, 12, 29) + hour), 0, "less than one day left");
    assert_eq!(server.days_left(day(2026, 12, 30)), 0, "the end itself");
    assert_eq!(server.days_left(day(2026, 12, 30) + hour), -1);
    assert_eq!(server.days_left(day(2027, 1, 1)), -2);
}

#[test]
fn a_block_that_is_no_certificate_gives_its_reason_and_the_other_blocks_stay() {
    let key = pem("PRIVATE KEY", b"not a key at all");
    let broken = pem("CERTIFICATE", b"not DER");
    let text = format!("{key}{broken}{}", chain());
    let found = certificates(text.as_bytes());
    assert_eq!(found.len(), 4, "{found:?}");
    assert_eq!(found[0], Err("block 1 is a PRIVATE KEY, not a certificate".to_owned()));
    assert_eq!(found[1], Err("block 2 is not an X.509 certificate".to_owned()));
    assert!(found[2].is_ok() && found[3].is_ok());

    assert_eq!(certificates(b""), [Err("the value has no PEM certificate".to_owned())]);
    assert_eq!(certificates(b"\x30\x82\x01\x00 DER"), [Err("the value has no PEM certificate".to_owned())]);
    let cut = "-----BEGIN CERTIFICATE-----\nMIIB\n";
    assert_eq!(certificates(cut.as_bytes()), [Err("block 1 is not a PEM block".to_owned())]);
    let bad_base64 = format!("-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n{}", chain());
    let found = certificates(bad_base64.as_bytes());
    assert_eq!(found.len(), 3, "the blocks after a bad one: {found:?}");
    assert_eq!(found[0], Err("block 1 is not a PEM block".to_owned()));

    let many = pem("CERTIFICATE", b"x").repeat(100);
    assert_eq!(certificates(many.as_bytes()).len(), MAX_BLOCKS);
}
