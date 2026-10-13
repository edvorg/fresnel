//! The options of the `watch` command.

use std::path::PathBuf;
use std::time::Duration;

use crate::contexts::{Picker, Selection};

pub(super) const USAGE: &str = "\
usage: fresnel-cli watch [options]

With no context, the command lists the contexts and connects to nothing. With a context, it
reads the objects of each kind of that cluster, as the app does. Of a Secret and of each
other kind with no detail view yet, it reads the metadata only. It changes nothing.

options:
  --context <name>      watch this context; the option can repeat
  --all                 watch each context
  --kubeconfig <file>   read this file in place of the kubeconfig files of the user;
                        the option can repeat
  --interval <seconds>  the time between two reports (default 5)
  --for <seconds>       stop after this time; with no value, the command runs until Ctrl+C";

const MAX_INTERVAL: u64 = 3600;
const MAX_TIME: u64 = 7 * 24 * 3600;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Options {
    /// Empty: the kubeconfig files of the user.
    pub(super) kubeconfig: Vec<PathBuf>,
    pub(super) selection: Selection,
    pub(super) interval: Duration,
    /// The run time. `None`: the command runs until the user stops it.
    pub(super) time: Option<Duration>,
}

/// Reads the options. The error is a text for the user.
pub(super) fn parse(arguments: &[String]) -> Result<Options, String> {
    let (mut interval, mut time) = (Duration::from_secs(5), None);
    let mut picker = Picker::default();
    let mut rest = arguments.iter();
    while let Some(option) = rest.next() {
        let mut value = || rest.next().ok_or_else(|| format!("the option {option} needs a value"));
        match option.as_str() {
            "--all" => picker.all(),
            "--context" => picker.context(value()?),
            "--kubeconfig" => picker.kubeconfig(value()?),
            "--interval" => interval = seconds(option, value()?, MAX_INTERVAL)?,
            "--for" => time = Some(seconds(option, value()?, MAX_TIME)?),
            other => return Err(format!("`{other}` is not an option of the command")),
        }
    }
    let (kubeconfig, selection) = picker.finish()?;
    Ok(Options { kubeconfig, selection, interval, time })
}

fn seconds(option: &str, text: &str, max: u64) -> Result<Duration, String> {
    let seconds = text.parse().ok().filter(|seconds| (1..=max).contains(seconds));
    let seconds = seconds.ok_or_else(|| format!("the value of {option} is a number from 1 to {max}, not `{text}`"))?;
    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Result<Options, String> {
        parse(&line.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
    }

    #[test]
    fn no_option_lists_the_contexts_and_each_option_sets_its_value() {
        let options = parsed("").expect("good options");
        assert_eq!((&options.selection, options.kubeconfig.len()), (&Selection::List, 0));
        assert_eq!((options.interval, options.time), (Duration::from_secs(5), None), "no end by itself");

        let options = parsed("--context a --kubeconfig /k/one --interval 2 --for 600").expect("good options");
        assert_eq!(options.selection, Selection::Names(vec!["a".to_owned()]));
        assert_eq!(options.kubeconfig, [PathBuf::from("/k/one")]);
        assert_eq!((options.interval, options.time), (Duration::from_secs(2), Some(Duration::from_secs(600))));
        assert_eq!(parsed("--all").expect("good options").selection, Selection::All);
    }

    #[test]
    fn a_bad_option_gives_its_reason() {
        let error = |line: &str| parsed(line).expect_err("bad options");
        assert_eq!(error("--once"), "`--once` is not an option of the command");
        assert_eq!(error("--for"), "the option --for needs a value");
        assert_eq!(error("--all --context a"), "use --all or --context, not the two");
        assert_eq!(error("--interval 0"), "the value of --interval is a number from 1 to 3600, not `0`");
        assert_eq!(error("--for soon"), "the value of --for is a number from 1 to 604800, not `soon`");
    }
}
