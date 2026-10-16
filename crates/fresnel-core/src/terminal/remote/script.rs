//! The shell commands of a terminal: the start of a shell with the mark of its session, and
//! the end of each process that has the mark.
//!
//! A WebSocket close, a reset and the end of the input leave the shell of an exec in the pod,
//! also with kubectl (terminal spike, X11). So the start puts a mark into the environment of
//! the shell, which each of its children inherits, and the close of a tab sends a hangup to
//! each process with the mark through a second exec. A mark in place of a process ID finds
//! the jobs in the background too, and needs no output on the screen of the user.

use std::fmt;

/// The name of the variable of the mark.
const MARK: &str = "FRESNEL_TERMINAL";
/// The value of `TERM`: the terminal model answers as xterm with 256 colors.
const TERM: &str = "xterm-256color";

/// The mark of one session: 32 hex digits, so no mark is the start of an other one, and a
/// script needs no quotes for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Mark(String);

impl Mark {
    pub(super) fn new() -> Self {
        Self(format!("{:016x}{:016x}", fastrand::u64(..), fastrand::u64(..)))
    }
}

impl fmt::Display for Mark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{MARK}={}", self.0)
    }
}

/// The command that starts the first shell of `shells` that exists, with `TERM` and the mark.
/// `sh -c` exports the variables and replaces itself with the shell, whose environment then
/// has them from its start, as `/proc/<pid>/environ` shows. With no shell, it ends with 127
/// and a line on the screen.
pub(super) fn launch(shells: &[String], mark: &Mark) -> Vec<String> {
    let list: Vec<String> = shells.iter().map(|shell| quote(shell)).collect();
    let list = list.join(" ");
    let script = format!(
        "export TERM={TERM} {mark}; for shell in {list}; do [ -x \"$shell\" ] && exec \"$shell\"; done; \
         echo \"fresnel: the container has none of the shells of the settings:\" {list} >&2; exit 127"
    );
    ["sh", "-c", &script].map(str::to_owned).to_vec()
}

/// The command of a node shell: the launch in the namespaces of the process 1 of the node.
/// The pod shares the process namespace of the node, so its process 1 is the one of the node.
pub(super) fn on_node(shells: &[String], mark: &Mark) -> Vec<String> {
    let enter = ["nsenter", "--target", "1", "--mount", "--uts", "--ipc", "--net", "--pid", "--"];
    enter.iter().map(|word| (*word).to_owned()).chain(launch(shells, mark)).collect()
}

/// The command of the second exec: a hangup to each process with the mark, and after 1 s a
/// kill to each one that still runs, such as a process of `nohup`. It prints the count of
/// the processes that got the hangup. The exec itself has no mark: the runtime gives it the
/// environment of the container, and the mark is in no spec.
pub(super) fn end(mark: &Mark) -> Vec<String> {
    let script = format!(
        "m={mark}; pids() {{ for p in /proc/[0-9]*; do grep -q \"$m\" \"$p/environ\" 2>/dev/null && echo \"${{p#/proc/}}\"; done; }}; \
         n=0; for i in $(pids); do [ \"$i\" = \"$$\" ] || {{ kill -HUP \"$i\" 2>/dev/null && n=$((n+1)); }}; done; \
         [ \"$n\" -gt 0 ] && sleep 1 && for i in $(pids); do [ \"$i\" = \"$$\" ] || kill -KILL \"$i\" 2>/dev/null; done; \
         echo \"$n\""
    );
    ["sh", "-c", &script].map(str::to_owned).to_vec()
}

/// A word in single quotes for `sh`: a quote inside ends the quotes, adds itself and opens
/// new ones, so a path of the settings can hold each character.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_tries_each_shell_in_its_sequence_with_the_mark_and_quotes_each_path() {
        let mark = Mark("0123456789abcdef0123456789abcdef".to_owned());
        let shells = ["/bin/bash".to_owned(), "/opt/it's/sh".to_owned()];
        let command = launch(&shells, &mark);
        assert_eq!(command[..2], ["sh", "-c"]);
        let expected = "export TERM=xterm-256color FRESNEL_TERMINAL=0123456789abcdef0123456789abcdef; \
             for shell in '/bin/bash' '/opt/it'\\''s/sh'; do [ -x \"$shell\" ] && exec \"$shell\"; done; \
             echo \"fresnel: the container has none of the shells of the settings:\" '/bin/bash' '/opt/it'\\''s/sh' >&2; exit 127";
        assert_eq!(command[2], expected);
        let node = on_node(&shells, &mark);
        assert_eq!(node[..10], ["nsenter", "--target", "1", "--mount", "--uts", "--ipc", "--net", "--pid", "--", "sh"]);
    }

    #[test]
    fn a_mark_has_32_hex_digits_and_the_end_finds_it_in_each_environment() {
        let mark = Mark::new();
        assert_eq!(mark.0.len(), 32);
        assert!(mark.0.chars().all(|char| char.is_ascii_hexdigit()));
        assert_ne!(mark, Mark::new());
        let script = &end(&mark)[2];
        assert!(script.starts_with(&format!("m=FRESNEL_TERMINAL={}; pids()", mark.0)));
        assert!(script.contains("kill -HUP") && script.contains("kill -KILL"));
    }
}
