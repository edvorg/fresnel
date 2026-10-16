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
    use super::super::tests::core;
    use super::*;
    use crate::cluster::ClusterId;
    use crate::config::Value;
    use crate::store::ObjectKey;
    use crate::terminal::{Options, Program, TerminalRequest};

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

    #[tokio::test]
    async fn a_close_removes_one_terminal_and_the_settings_give_each_one_its_history() {
        let core = core();
        let (first, second) = (view(1), view(2));
        first.terminal.feed(&b"line\r\n".repeat(50));
        core.inner.state().terminals = Arc::from([first.clone(), second]);

        let mut config = Config::default();
        config.values.insert(Setting::TerminalScrollback, Value::Number(20));
        core.inner.follow_terminal_settings(&config);
        let mut frame = crate::terminal::Frame::default();
        first.terminal.frame(&mut frame, &crate::terminal::Palette::DARK);
        assert_eq!(frame.history, 20);

        core.inner.close_terminal(2);
        let ids: Vec<u64> = core.view().terminals.iter().map(|view| view.id).collect();
        assert_eq!(ids, [1]);
        core.inner.close_terminal(7);
        assert_eq!(core.view().terminals.len(), 1, "an other identity removes nothing");
    }
}
