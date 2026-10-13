//! What each test of the program needs for a kubeconfig on a fake API server.

use fresnel_testkit::FakeApiServer;

/// The certificate of a fake server as the value of `certificate-authority-data`.
pub(crate) fn authority(server: &FakeApiServer) -> String {
    let certificate = base64(server.certificate_der());
    base64(format!("-----BEGIN CERTIFICATE-----\n{certificate}\n-----END CERTIFICATE-----\n").as_bytes())
}

/// Base64 with padding. The tests need it for one certificate, so they have no crate for it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().fold(0_u32, |group, byte| group << 8 | u32::from(*byte)) << (8 * (3 - chunk.len()));
        for position in 0..4 {
            let symbol = ALPHABET[(group >> (18 - 6 * position) & 63) as usize];
            text.push(if position <= chunk.len() { char::from(symbol) } else { '=' });
        }
    }
    text
}
