//! The start of the core on Windows, as from Explorer: the environment of the process is
//! complete, so no login shell runs, and the bare name of the plugin in the kubeconfig finds
//! its file with an extension of `PATHEXT`, as `command: doctl` finds `doctl.exe`.

use fresnel_core::app::{CoreSettings, Health};
use fresnel_core::kubeconfig::AuthKind;
use fresnel_core::net::credential::CredentialState;
use fresnel_testkit::FakeApiServer;

use super::{App, PLUGIN, home, row, write_kubeconfig};

/// The plugin of the tests on Windows: a batch file that writes a line for each run and
/// gives the token `<FAKE_PLUGIN_TOKEN>-1`.
pub(super) const PLUGIN_SCRIPT: &str = concat!(
    "@echo off\r\n",
    r#"echo run>>"%FAKE_PLUGIN_STATE%\runs""#,
    "\r\n",
    r#"echo {"apiVersion":"client.authentication.k8s.io/v1beta1","kind":"ExecCredential","#,
    r#""status":{"token":"%FAKE_PLUGIN_TOKEN%-1"}}"#,
    "\r\n",
);

#[tokio::test(flavor = "multi_thread")]
async fn the_core_connects_with_the_environment_of_explorer_and_finds_the_plugin_by_pathext() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("explorer");
    write_kubeconfig(&home, "fake", &[("fake", server.addr(), &server, true)]);
    let app = App::started(&home, CoreSettings::default());
    let view = app.until_health("fake", Health::Live).await;
    let fake = row(&view, "fake");
    assert_eq!((fake.auth.clone(), fake.reason()), (Some(AuthKind::Plugin(PLUGIN.to_owned())), None));
    let credential = fake.status().and_then(|status| status.credential.clone()).expect("a credential");
    assert!(matches!(credential.state, CredentialState::Valid { .. }), "{credential:?}");
    assert_eq!((credential.plugin_runs, home.read("state/runs").lines().count()), (1, 1), "one run of the plugin");
    // A channel can send its first light request before the plugin gave the token.
    let tokens: Vec<String> = server.requests().iter().filter_map(|seen| seen.bearer().map(str::to_owned)).collect();
    assert!(!tokens.is_empty() && tokens.iter().all(|token| token == "core-1"), "{tokens:?}");

    let texts = app.texts_of("app");
    let after_the_settings =
        ["the environment of the process is complete; no login shell runs", "read 1 context from 1 kubeconfig file"];
    assert_eq!(texts[1..], after_the_settings);
}
