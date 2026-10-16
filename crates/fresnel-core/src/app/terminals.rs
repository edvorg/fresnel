//! The terminals of the dock (plan section 12.8): the core holds each one, the view gets a
//! copy of the list, and a close removes one.
//!
//! A terminal stays after the end of its session, with its screen and the final state, until
//! the user closes it: the end of a shell is no reason to lose its output.

use std::sync::Arc;

use super::Inner;
use crate::config::Config;
use crate::config::table::Setting;
use crate::terminal::{Terminal, TerminalView};

impl Inner {
    /// Removes a terminal of the dock.
    pub(super) fn close_terminal(&self, id: u64) {
        let closed = {
            let mut state = self.state();
            let kept: Arc<[TerminalView]> = state.terminals.iter().filter(|view| view.id != id).cloned().collect();
            std::mem::replace(&mut state.terminals, kept)
        };
        // The last handle of the terminal drops after the lock.
        drop(closed);
        self.repaint.request();
    }

    /// Gives each terminal the lines of history of the settings: they act at once.
    pub(super) fn follow_terminal_settings(&self, config: &Config) {
        let lines = usize::try_from(config.number(Setting::TerminalScrollback)).unwrap_or(usize::MAX);
        let terminals: Vec<Terminal> = self.state().terminals.iter().map(|view| view.terminal.clone()).collect();
        for terminal in terminals {
            terminal.set_scrollback(lines);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use fresnel_net::WallClock;
    use fresnel_testkit::Home;
    use tokio::runtime::Handle;

    use super::super::{Command, Core, CoreParts, CoreSettings, Startup};
    use super::*;
    use crate::cluster::ClusterId;
    use crate::config::{Edit, EditValue};
    use crate::journal::Journal;
    use crate::store::ObjectKey;
    use crate::terminal::{Frame, Options, Palette, Program, TerminalRequest};

    fn view(id: u64) -> TerminalView {
        let request = TerminalRequest {
            cluster: ClusterId::new("ams"),
            pod: ObjectKey::new("shop", "web-1"),
            uid: "uid-1".into(),
            container: "app".into(),
            program: Program::Shell,
        };
        TerminalView { id, request, terminal: Terminal::new(Options::default()) }
    }

    fn history(terminal: &Terminal) -> usize {
        let mut frame = Frame::default();
        terminal.frame(&mut frame, &Palette::DARK);
        frame.history
    }

    /// The commands of the views go through the queue of the started core: a save of the
    /// settings view gives each terminal its history at once, and a close removes the
    /// terminal of its identity only. The commands run in their sequence.
    #[tokio::test]
    async fn a_save_of_the_settings_and_a_close_reach_the_terminals_of_the_started_core() {
        let home = Home::new("terminals", &["kubeconfig.yaml"]).expect("a home folder");
        home.write("work/kubeconfig.yaml", "contexts:\n- {name: a, context: {cluster: none}}\n").expect("a file");
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
        let (first, second) = (view(1), view(2));
        first.terminal.feed(&b"line\r\n".repeat(50));
        let ready = async {
            while core.view().startup != Startup::Ready {
                tokio::task::yield_now().await;
            }
            core.inner.state().terminals = Arc::from([first.clone(), second]);
            let lines = Edit::new(&["terminal", "scrollback_lines"], EditValue::Number(20));
            core.send(Command::EditSettings(vec![lines]));
            core.send(Command::CloseTerminal(7));
            core.send(Command::CloseTerminal(2));
            while core.view().terminals.len() != 1 {
                tokio::task::yield_now().await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), ready).await.expect("the commands ran");
        core.stop();
        let ids: Vec<u64> = core.view().terminals.iter().map(|view| view.id).collect();
        assert_eq!(ids, [1], "an other identity removes nothing");
        assert_eq!(history(&first.terminal), 20, "the lines of history of the saved settings");
    }
}
