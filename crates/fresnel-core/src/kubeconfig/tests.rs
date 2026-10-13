use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::json;

use super::material::Material;
use super::*;

/// A folder for the files of one test. Its drop removes it.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        static NUMBER: AtomicU32 = AtomicU32::new(0);
        let name = format!("fresnel-kubeconfig-{}-{}", std::process::id(), NUMBER.fetch_add(1, Ordering::Relaxed));
        let folder = std::env::temp_dir().join(name);
        std::fs::create_dir(&folder).expect("a new folder");
        Self(folder)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("a write to the test folder");
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("the test folder goes");
    }
}

const FIRST: &str = r#"
apiVersion: v1
kind: Config
current-context: ams
clusters:
- name: ams
  cluster:
    server: https://ams.example.test
    certificate-authority-data: QUJD
    tls-server-name: kubernetes
    extensions:
    - name: client.authentication.k8s.io/exec
      extension: {audience: ams}
- {name: nyc, cluster: {server: "https://[2001:db8::1]:6443/k8s/clusters/c1/", certificate-authority: ca/nyc.pem}}
users:
- name: plugin-user
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: doctl
      args: [kubernetes, cluster, kubeconfig, exec-credential]
      env: [{name: DIGITALOCEAN_CONTEXT, value: work}]
      installHint: Install doctl.
      provideClusterInfo: true
      interactiveMode: IfAvailable
- name: certificate-user
  user: {client-certificate: certs/user.pem, client-key-data: a2V5LW9mLXRoZS11c2Vy}
contexts:
- {name: ams, context: {cluster: ams, user: plugin-user, namespace: apps}}
- {name: nyc, context: {cluster: nyc, user: certificate-user}}
- {name: open, context: {cluster: ams}}
"#;

const SECOND: &str = r#"
clusters:
- {name: sgp, cluster: {server: "https://sgp.example.test:8443", certificate-authority-data: QUJD}}
- {name: ams, cluster: {server: "https://other.example.test", certificate-authority-data: QUJD}}
users:
- {name: plugin-user, user: {client-certificate-data: QUJD, client-key-data: QUJD}}
contexts:
- {name: sgp, context: {cluster: sgp, user: plugin-user}}
- {name: ams, context: {cluster: sgp}}
---
current-context: sgp
contexts: null
users:
"#;

fn access(kubeconfig: &Kubeconfig, name: &str) -> Access {
    let context = kubeconfig.contexts.iter().find(|context| context.name == name).expect("the context");
    context.access.clone().unwrap_or_else(|error| panic!("the context {name} has an error: {error}"))
}

#[test]
fn the_load_lists_each_context_of_each_file_and_the_first_file_wins() {
    let folder = Folder::new();
    let first = folder.write("first.yaml", FIRST);
    let broken = folder.write("broken.yaml", "clusters:\n- name: x\n  cluster: {server: [secret-in-the-file\n");
    let second = folder.write("second.yaml", SECOND);
    let absent = folder.0.join("absent.yaml");
    let empty = folder.write("empty.yaml", "# no entry\n");
    let sources = [first.clone(), broken.clone(), absent.clone(), second.clone(), empty];
    let kubeconfig = load(&sources);

    let names: Vec<&str> = kubeconfig.contexts.iter().map(|context| context.name.as_str()).collect();
    assert_eq!(names, ["ams", "nyc", "open", "sgp"]);
    assert_eq!((kubeconfig.sources.as_slice(), kubeconfig.absent.as_slice()), (&sources[..], &[absent][..]));
    assert_eq!(kubeconfig.current.as_deref(), Some("ams"), "the first file that has the value wins");
    let (ams, sgp) = (&kubeconfig.contexts[0], &kubeconfig.contexts[3]);
    assert_eq!((ams.namespace.as_deref(), &ams.source, &sgp.source), (Some("apps"), &first, &second));
    assert_eq!(access(&kubeconfig, "ams").server.host, "ams.example.test", "the cluster of the first file");
    // The user of the second file has the name of a user of the first file.
    assert_eq!(access(&kubeconfig, "sgp").auth(), AuthKind::Plugin("doctl".to_owned()));

    let problem = |path: &PathBuf, text: &str| Problem { path: path.clone(), text: text.to_owned() };
    let expected = [
        problem(&broken, "the file is not a kubeconfig: line 3, column 21"),
        problem(&second, "the cluster `ams` has an earlier entry, which wins"),
        problem(&second, "the user `plugin-user` has an earlier entry, which wins"),
        problem(&second, "the context `ams` has an earlier entry, which wins"),
    ];
    assert_eq!(kubeconfig.problems, expected);
}

#[test]
fn a_context_gives_the_server_and_the_credential_of_its_entries() {
    let folder = Folder::new();
    let path = folder.write("config", FIRST);
    let kubeconfig = load(&[path]);

    let ams = access(&kubeconfig, "ams");
    let server = Server { host: "ams.example.test".to_owned(), port: 443, base_path: String::new() };
    assert_eq!((&ams.server, ams.tls_server_name.as_deref()), (&server, Some("kubernetes")));
    assert_eq!(
        (ams.server.to_string().as_str(), ams.auth()),
        ("ams.example.test:443", AuthKind::Plugin("doctl".into()))
    );
    assert!(ams.authority == Material::Inline(b"ABC".to_vec()) && ams.identity.is_none());
    let plugin = ams.plugin().expect("a plugin");
    assert_eq!(plugin.args, ["kubernetes", "cluster", "kubeconfig", "exec-credential"]);
    assert_eq!(plugin.env, [("DIGITALOCEAN_CONTEXT".to_owned(), "work".to_owned())]);
    assert_eq!(plugin.api_version.as_deref(), Some("client.authentication.k8s.io/v1beta1"));
    assert_eq!(plugin.install_hint.as_deref(), Some("Install doctl."));
    assert_eq!((plugin.folder.as_deref(), &plugin.cluster), (Some(folder.0.as_path()), &None));
    // The plugin asks for the cluster data. `kubectl` gives these fields.
    let expected = json!({
        "server": "https://ams.example.test",
        "certificate-authority-data": "LS0t",
        "tls-server-name": "kubernetes",
        "config": { "audience": "ams" },
    });
    assert_eq!(ams.plugin_for_connect(b"---").expect("a plugin").cluster, Some(expected));

    let nyc = access(&kubeconfig, "nyc");
    let server = Server { host: "2001:db8::1".to_owned(), port: 6443, base_path: "/k8s/clusters/c1".to_owned() };
    assert_eq!((&nyc.server, nyc.auth(), &nyc.tls_server_name), (&server, AuthKind::Certificate, &None));
    assert!(nyc.authority == Material::File(folder.0.join("ca/nyc.pem")), "a path starts from the folder of the file");
    let identity = nyc.identity.as_ref().expect("a client certificate");
    assert!(identity.certificate == Material::File(folder.0.join("certs/user.pem")));
    assert!(identity.key == Material::Inline(b"key-of-the-user".to_vec()));
    assert!(nyc.plugin().is_none() && nyc.plugin_for_connect(b"").is_none());

    assert_eq!(access(&kubeconfig, "open").auth(), AuthKind::Anonymous);
}

/// Each line: the name of the context, its cluster entry, its user entry and the error.
const REFUSED: [(&str, &str, &str, &str); 17] = [
    ("no-server", "{certificate-authority-data: QUJD}", "{}", "the cluster entry `no-server` has no server"),
    (
        "http",
        "{server: 'http://a.test', certificate-authority-data: QUJD}",
        "{}",
        "the server URL `http://a.test` is not an https URL; the app does not support that",
    ),
    (
        "bad-url",
        "{server: 'https://a b', certificate-authority-data: QUJD}",
        "{}",
        "the server URL `https://a b` is not a URL",
    ),
    (
        "no-ca",
        "{server: 'https://a.test'}",
        "{}",
        "the cluster entry `no-ca` has no certificate authority; the app does not support that yet",
    ),
    (
        "bad-ca",
        "{server: 'https://a.test', certificate-authority-data: '%%'}",
        "{}",
        "the certificate authority data of the kubeconfig is not base64",
    ),
    (
        "insecure",
        "{server: 'https://a.test', insecure-skip-tls-verify: true}",
        "{}",
        "the cluster entry `insecure` skips the check of the server certificate; the app does not support that yet",
    ),
    (
        "proxy",
        "{server: 'https://a.test', certificate-authority-data: QUJD, proxy-url: 'http://p.test'}",
        "{}",
        "the cluster entry `proxy` uses a proxy; the app does not support that yet",
    ),
    (
        "token",
        "",
        "{token: secret-token-value}",
        "the user entry `token` has a static token; the app does not support that yet",
    ),
    (
        "token-file",
        "",
        "{tokenFile: /var/run/token}",
        "the user entry `token-file` has a static token; the app does not support that yet",
    ),
    (
        "password",
        "",
        "{username: admin, password: secret-password-value}",
        "the user entry `password` has a password; the app does not support that yet",
    ),
    (
        "provider",
        "",
        "{auth-provider: {name: oidc}}",
        "the user entry `provider` has an auth provider; the app does not support that yet",
    ),
    (
        "as",
        "",
        "{as: root, client-certificate-data: QUJD, client-key-data: QUJD}",
        "the user entry `as` has a user to act as; the app does not support that yet",
    ),
    (
        "no-key",
        "",
        "{client-certificate-data: QUJD}",
        "the user entry `no-key` has a client certificate and no client key",
    ),
    (
        "no-certificate",
        "",
        "{client-key: key.pem}",
        "the user entry `no-certificate` has a client key and no client certificate",
    ),
    ("no-command", "", "{exec: {args: [token]}}", "the exec entry of the user `no-command` has no command"),
    (
        "terminal",
        "",
        "{exec: {command: login, interactiveMode: Always}}",
        "the credential plugin of the user `terminal` needs a terminal, and the app has none",
    ),
    (
        "bad-key",
        "",
        "{client-certificate-data: QUJD, client-key-data: 'secret key'}",
        "the client key data of the kubeconfig is not base64",
    ),
];

#[test]
fn an_entry_that_the_app_cannot_use_gives_its_reason_and_no_secret() {
    const GOOD_CLUSTER: &str = "{server: 'https://a.test', certificate-authority-data: QUJD}";
    let mut text = String::from("clusters:\n- name: no-body\nusers:\n- name: no-body\ncontexts:\n- name: no-body\n");
    text.push_str("- {name: no-cluster, context: {user: none}}\n- {name: lost-cluster, context: {cluster: lost}}\n");
    text.push_str("- {name: lost-user, context: {cluster: good, user: lost}}\n");
    text.push_str("- {name: empty-cluster, context: {cluster: no-body}}\n");
    text.push_str("- {name: empty-user, context: {cluster: good, user: no-body}}\n");
    let mut clusters = format!("- {{name: good, cluster: {GOOD_CLUSTER}}}\n");
    let mut users = String::new();
    for (name, cluster, user, _) in REFUSED {
        let cluster = if cluster.is_empty() { GOOD_CLUSTER } else { cluster };
        clusters.push_str(&format!("- {{name: {name}, cluster: {cluster}}}\n"));
        users.push_str(&format!("- {{name: {name}, user: {user}}}\n"));
        text.push_str(&format!("- {{name: {name}, context: {{cluster: {name}, user: {name}}}}}\n"));
    }
    let folder = Folder::new();
    let kubeconfig =
        load(&[folder.write("config", &text.replacen("users:\n", &format!("{clusters}users:\n{users}"), 1))]);
    assert_eq!(kubeconfig.problems, []);

    let error = |name: &str| {
        let context = kubeconfig.contexts.iter().find(|context| context.name == name).expect("the context");
        let error = context.access.clone().expect_err("an entry that the app cannot use");
        assert_eq!(error.class(), crate::net::ErrorClass::Invalid, "{name}");
        error.to_string()
    };
    assert_eq!(error("no-body"), "the context entry has no body");
    assert_eq!(error("no-cluster"), "no file defines the cluster ``");
    assert_eq!(error("lost-cluster"), "no file defines the cluster `lost`");
    assert_eq!(error("lost-user"), "no file defines the user `lost`");
    assert_eq!(error("empty-cluster"), "the cluster entry `no-body` has no body");
    assert_eq!(error("empty-user"), "the user entry `no-body` has no body");
    for (name, _, _, expected) in REFUSED {
        assert_eq!(error(name), expected);
    }
    assert_eq!(kubeconfig.contexts.len(), REFUSED.len() + 6, "a context with an error stays in the list");
    let shown = format!("{kubeconfig:?}");
    for secret in ["secret-token-value", "secret-password-value", "secret key", "QUJD"] {
        assert!(!shown.contains(secret), "the debug text shows `{secret}`");
    }
}

#[test]
fn the_debug_text_of_a_good_context_shows_no_certificate_and_no_key() {
    let folder = Folder::new();
    let kubeconfig = load(&[folder.write("config", FIRST)]);
    let shown = format!("{:?}", kubeconfig.contexts);
    assert!(shown.contains("ams.example.test") && shown.contains("Plugin(\"doctl\")"), "{shown}");
    for secret in ["key-of-the-user", "a2V5", "QUJD", "DIGITALOCEAN_CONTEXT", "[107, 101"] {
        assert!(!shown.contains(secret), "the debug text shows `{secret}`: {shown}");
    }
}
