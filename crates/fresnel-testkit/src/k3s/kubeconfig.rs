//! The kubeconfig of a test cluster, with the server address that a test needs.

use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use crate::KitError;

/// The server address in the kubeconfig that k3s writes. It is the supervisor port, which
/// the test kit does not publish.
const K3S_SERVER: &str = "server: https://127.0.0.1:6443";

/// Gives the kubeconfig text with `server` as the address of the API server.
pub(super) fn with_server(k3s_text: &str, server: SocketAddr) -> Result<String, KitError> {
    if k3s_text.matches(K3S_SERVER).count() != 1 {
        return Err(KitError::new(format!("the kubeconfig of k3s does not have the line `{K3S_SERVER}` one time")));
    }
    Ok(k3s_text.replace(K3S_SERVER, &format!("server: https://{server}")))
}

/// Writes the file for its owner only, because it holds the key of the cluster admin.
pub(super) fn write(path: &Path, text: &str) -> Result<(), KitError> {
    let written = write_private(path, text);
    written.map_err(|error| KitError::because(format!("cannot write {}", path.display()), error))
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const K3S_TEXT: &str = "clusters:\n- cluster:\n    certificate-authority-data: QUJD\n    server: https://127.0.0.1:6443\n  name: default\n";

    #[test]
    fn the_server_line_gets_the_new_address_and_no_other_line_changes() {
        let text = with_server(K3S_TEXT, "127.0.0.1:50123".parse().expect("an address")).expect("a kubeconfig");
        assert_eq!(text, K3S_TEXT.replace("127.0.0.1:6443", "127.0.0.1:50123"));
    }

    #[test]
    fn a_text_with_no_server_line_or_with_two_is_an_error() {
        let address = "127.0.0.1:50123".parse().expect("an address");
        assert!(with_server("clusters: []\n", address).is_err());
        assert!(with_server(&K3S_TEXT.repeat(2), address).is_err());
    }

    #[test]
    fn the_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!("fresnel-test-kubeconfig-{}", std::process::id()));
        write(&path, "text").expect("write");
        let mode = fs::metadata(&path).expect("metadata").permissions().mode();
        let text = fs::read_to_string(&path).expect("read");
        fs::remove_file(&path).expect("remove");
        assert_eq!((mode & 0o777, text.as_str()), (0o600, "text"));
    }
}
