use std::sync::mpsc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_net::env::ProcessEnv;
use fresnel_net::task::TaskReport;
use fresnel_net::{NetSettings, WallClock};
use fresnel_testkit::{FakeApiServer, Home};
use tokio::runtime::Handle;

use super::*;
use crate::cluster::Registry;
use crate::journal::{Entry, Sink};
use crate::kubeconfig;

/// Sends the text of each entry to the test.
struct Texts(mpsc::Sender<String>);

impl Sink for Texts {
    fn write(&mut self, entry: &Entry) {
        // After the end of the test, an entry can come before the task stops.
        if let Err(error) = self.0.send(entry.text.clone()) {
            eprintln!("an entry after the end of the test: {}", error.0);
        }
    }
}

/// A kubeconfig with one cluster on the fake server and a user with no credential.
fn kubeconfig(home: &Home, server: &FakeApiServer) -> std::path::PathBuf {
    let certificate = STANDARD.encode(server.certificate_der());
    let pem = format!("-----BEGIN CERTIFICATE-----\n{certificate}\n-----END CERTIFICATE-----\n");
    let cluster =
        format!("{{server: 'https://{}', certificate-authority-data: {}}}", server.addr(), STANDARD.encode(pem));
    let text = format!(
        "clusters:\n- {{name: fake, cluster: {cluster}}}\ncontexts:\n- {{name: fake, context: {{cluster: fake}}}}\n"
    );
    home.write("work/kubeconfig.yaml", &text).expect("the kubeconfig");
    home.path("work/kubeconfig.yaml")
}

/// The race that CI found: on a near server, the channels were live before the task read
/// its first state, and the journal had no entry for the start of the connect.
#[tokio::test(flavor = "multi_thread")]
async fn the_journal_has_the_start_of_the_connect_also_when_the_channels_were_live_before_the_task() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = Home::new("state-watch", &[]).expect("a home folder");
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
    let registry = Registry::new(tasks.clone(), Handle::current(), ProcessEnv::default(), NetSettings::default());
    registry.set_contexts(kubeconfig::load(&[kubeconfig(&home, &server)]).contexts);
    let id = ClusterId::new("fake");
    let link = registry.connect(&id).expect("a link");
    let mut changes = link.net.changes();
    while Health::of_status(&link.net.status()) != Health::Live {
        changes.next().await;
    }

    let (sender, texts) = mpsc::channel();
    let journal = Journal::with_sink(10, WallClock::system(), Texts(sender));
    let _watch = start(&tasks, &id, link.net.clone(), journal, Repaint::new());
    let next = || texts.recv_timeout(Duration::from_secs(10)).expect("an entry");
    assert_eq!([next(), next()], ["the channels connect", "live: a request got an answer on each channel"]);
}
