use std::time::Duration;

use fresnel_net::WallClock;
use fresnel_testkit::Home;
use tokio::runtime::Handle;

use super::super::{Command, Core, CoreParts, CoreSettings, CoreView, Startup};
use super::*;
use crate::config::{Edit, EditValue};
use crate::journal::Journal;
use crate::store::ObjectKey;
use crate::terminal::{Frame, Palette};

fn request(cluster: &str, program: Program) -> TerminalRequest {
    TerminalRequest {
        cluster: ClusterId::new(cluster),
        object: ObjectKey::new("shop", "web-1"),
        uid: "uid-1".into(),
        container: "app".into(),
        program,
    }
}

fn history(terminal: &Terminal) -> usize {
    let mut frame = Frame::default();
    terminal.frame(&mut frame, &Palette::DARK);
    frame.history
}

/// Waits until the view of the core meets the condition.
async fn until(core: &Core, met: impl Fn(&CoreView) -> bool) -> CoreView {
    let wait = async {
        loop {
            let view = core.view();
            if met(&view) {
                return view;
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(Duration::from_secs(10), wait).await.expect("the commands ran")
}

/// The commands of the views go through the queue of the started core. An open in a
/// cluster that is not connected, and a change of a read-only cluster, give a terminal that
/// failed with the reason. A save of the settings view gives each terminal its history at
/// once, and a close removes the terminal of its identity only.
#[tokio::test]
async fn an_open_a_save_of_the_settings_and_a_close_reach_the_terminals_of_the_started_core() {
    let home = Home::new("terminals", &["kubeconfig.yaml"]).expect("a home folder");
    home.write("work/kubeconfig.yaml", "contexts:\n- {name: a, context: {cluster: none}}\n").expect("a file");
    home.write("state/config.toml", "version = 1\n[clusters.prod]\nread_only = true\n").expect("a file");
    let core = Core::new(CoreParts {
        runtime: Handle::current(),
        base_env: home.finder_env(),
        journal: Journal::new(2000, WallClock::system()),
        state_file: home.path("state/state.json"),
        config_file: home.path("state/config.toml"),
        downloads: home.path("Downloads"),
        detectors: Vec::new(),
        settings: CoreSettings::default(),
    });
    core.start(|| {});
    until(&core, |view| view.startup == Startup::Ready && view.config.read_only(&ClusterId::new("prod"))).await;
    core.send(Command::OpenTerminal(request("a", Program::Shell)));
    core.send(Command::OpenTerminal(request("prod", Program::NodeShell)));
    let view = until(&core, |view| view.terminals.len() == 2).await;
    let states: Vec<SessionState> = view.terminals.iter().map(|view| view.terminal.state()).collect();
    assert_eq!(states[0], SessionState::Failed { reason: "a is not connected".to_owned() });
    assert!(matches!(&states[1], SessionState::Failed { reason } if reason.contains("read-only")), "{states:?}");
    let first = view.terminals[0].terminal.clone();
    first.feed(&b"line\r\n".repeat(50));
    let lines = Edit::new(&["terminal", "scrollback_lines"], EditValue::Number(20));
    core.send(Command::EditSettings(vec![lines]));
    core.send(Command::CloseTerminal(7));
    core.send(Command::CloseTerminal(2));
    let view = until(&core, |view| view.terminals.len() == 1).await;
    core.stop();
    let ids: Vec<u64> = view.terminals.iter().map(|view| view.id).collect();
    assert_eq!(ids, [1], "an other identity removes nothing");
    assert_eq!(history(&first), 20, "the lines of history of the saved settings");
}
