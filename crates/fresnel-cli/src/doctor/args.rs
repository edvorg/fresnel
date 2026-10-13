//! The options of the `doctor` command.

use std::path::PathBuf;
use std::time::Duration;

pub(super) const USAGE: &str = "\
usage: fresnel-cli doctor [options]

With no context, the command lists the contexts and connects to nothing.

options:
  --context <name>           examine this context; the option can repeat
  --all                      examine each context
  --kubeconfig <file>        read this file in place of the kubeconfig files of the user;
                             the option can repeat
  --samples <count>          the connects of each type, with and with no hedge (default 20)
  --connect-limit <seconds>  the longest wait for one connect with no hedge (default 5)
  --idle-close <seconds>     also measure when the server closes an idle connection, and
                             wait this long at most
  --json <file>              the JSON report (default fresnel-doctor.json)";

const MAX_SAMPLES: usize = 1000;
const MAX_CONNECT_LIMIT: u64 = 60;
const MAX_IDLE_CLOSE: u64 = 3600;

/// Which contexts the command examines.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Selection {
    /// None: the command lists the contexts.
    List,
    All,
    Names(Vec<String>),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Options {
    /// Empty: the kubeconfig files of the user.
    pub(super) kubeconfig: Vec<PathBuf>,
    pub(super) selection: Selection,
    pub(super) samples: usize,
    pub(super) connect_limit: Duration,
    pub(super) idle_close: Option<Duration>,
    pub(super) json: PathBuf,
}

/// Reads the options. The error is a text for the user.
pub(super) fn parse(arguments: &[String]) -> Result<Options, String> {
    let mut options = Options {
        kubeconfig: Vec::new(),
        selection: Selection::List,
        samples: 20,
        connect_limit: Duration::from_secs(5),
        idle_close: None,
        json: PathBuf::from("fresnel-doctor.json"),
    };
    let (mut names, mut all) = (Vec::new(), false);
    let mut rest = arguments.iter();
    while let Some(option) = rest.next() {
        let mut value = || rest.next().ok_or_else(|| format!("the option {option} needs a value"));
        match option.as_str() {
            "--all" => all = true,
            "--context" => names.push(value()?.clone()),
            "--kubeconfig" => options.kubeconfig.push(PathBuf::from(value()?)),
            "--json" => options.json = PathBuf::from(value()?),
            "--samples" => options.samples = number(option, value()?, 0, MAX_SAMPLES)?,
            "--connect-limit" => options.connect_limit = seconds(option, value()?, MAX_CONNECT_LIMIT)?,
            "--idle-close" => options.idle_close = Some(seconds(option, value()?, MAX_IDLE_CLOSE)?),
            other => return Err(format!("`{other}` is not an option of the command")),
        }
    }
    options.selection = match (all, names.is_empty()) {
        (true, false) => return Err("use --all or --context, not the two".to_owned()),
        (true, true) => Selection::All,
        (false, false) => Selection::Names(names),
        (false, true) => Selection::List,
    };
    Ok(options)
}

fn number(option: &str, text: &str, min: usize, max: usize) -> Result<usize, String> {
    let number = text.parse().ok().filter(|number| (min..=max).contains(number));
    number.ok_or_else(|| format!("the value of {option} is a number from {min} to {max}, not `{text}`"))
}

fn seconds(option: &str, text: &str, max: u64) -> Result<Duration, String> {
    let max = usize::try_from(max).unwrap_or(usize::MAX);
    let seconds = number(option, text, 1, max)?;
    Ok(Duration::from_secs(u64::try_from(seconds).unwrap_or(u64::MAX)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(arguments: &[&str]) -> Result<Options, String> {
        parse(&arguments.iter().map(|argument| (*argument).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn no_option_lists_the_contexts_of_the_files_of_the_user() {
        let options = parsed(&[]).expect("good options");
        assert_eq!(options.selection, Selection::List);
        assert_eq!((options.kubeconfig.len(), options.samples, options.idle_close), (0, 20, None));
        assert_eq!(
            (options.connect_limit, options.json),
            (Duration::from_secs(5), PathBuf::from("fresnel-doctor.json"))
        );
    }

    #[test]
    fn each_option_sets_its_value() {
        let line = "--context a --kubeconfig /k/one --samples 3 --context b --kubeconfig two --connect-limit 2 \
                    --idle-close 150 --json /tmp/out.json";
        let options = parsed(&line.split_whitespace().collect::<Vec<_>>()).expect("good options");
        assert_eq!(options.selection, Selection::Names(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(options.kubeconfig, [PathBuf::from("/k/one"), PathBuf::from("two")]);
        assert_eq!((options.samples, options.connect_limit), (3, Duration::from_secs(2)));
        assert_eq!(
            (options.idle_close, options.json),
            (Some(Duration::from_secs(150)), PathBuf::from("/tmp/out.json"))
        );
        assert_eq!(parsed(&["--all"]).expect("good options").selection, Selection::All);
    }

    #[test]
    fn a_bad_option_gives_its_reason() {
        let error = |arguments: &[&str]| parsed(arguments).expect_err("bad options");
        assert_eq!(error(&["--verbose"]), "`--verbose` is not an option of the command");
        assert_eq!(error(&["--context"]), "the option --context needs a value");
        assert_eq!(error(&["--all", "--context", "a"]), "use --all or --context, not the two");
        assert_eq!(error(&["--samples", "many"]), "the value of --samples is a number from 0 to 1000, not `many`");
        assert_eq!(error(&["--samples", "1001"]), "the value of --samples is a number from 0 to 1000, not `1001`");
        assert_eq!(error(&["--connect-limit", "0"]), "the value of --connect-limit is a number from 1 to 60, not `0`");
        assert_eq!(error(&["--idle-close", "-5"]), "the value of --idle-close is a number from 1 to 3600, not `-5`");
    }
}
