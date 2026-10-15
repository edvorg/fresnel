//! The options of the `soak` command.

use std::path::PathBuf;

pub(super) const USAGE: &str = "\
usage: fresnel-cli soak --context <name> --kubeconfig <file> --report <folder> [options]

Runs the app core on one cluster: each store, the indexes, the metrics sampler and a log
view. Each minute, it records the state of each store, each gap between a store and the
API, the lag of the stores, the heap and the open connections, in soak.json and soak.txt of
the report folder. The exit status is 1 when a store shows \"live\" and is behind the API
for more than 20 s, or when the heap grows above its limit. It changes nothing in a cluster.

options:
  --context <name>        the cluster
  --kubeconfig <file>     the kubeconfig of the core, in place of the files of the user; the
                          option can repeat
  --compare <file>        a kubeconfig of the same context for the lists that each store is
                          compared with, for example one with no fault proxy (default: the
                          kubeconfig of the core)
  --report <folder>       the folder of the report, the journal and the files of the core
  --minutes <number>      the run time (default 60)
  --log <namespace/name>  a Deployment whose pods write lines \"soak <number>\": the soak
                          follows its log, and a number that does not come is a gap
  --memory-limit <MiB>    the most growth of the heap after the warm-up (default 64)
  --warm-up <minutes>     the minutes before the heap that the growth starts from (default 10)
  --faults <file>         a file to which a harness adds one line for each fault that it
                          makes; each minute of the report lists its new lines";

const MAX_MINUTES: u64 = 7 * 24 * 60;
const MAX_MEMORY_LIMIT: u64 = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Options {
    pub(super) context: String,
    pub(super) kubeconfig: Vec<PathBuf>,
    /// Empty: the kubeconfig of the core.
    pub(super) compare: Vec<PathBuf>,
    pub(super) report: PathBuf,
    pub(super) minutes: u64,
    /// The namespace and the name of the Deployment of the log view.
    pub(super) log: Option<(String, String)>,
    pub(super) memory_limit_mib: u64,
    pub(super) warm_up: u64,
    pub(super) faults: Option<PathBuf>,
}

/// Reads the options. The error is a text for the user.
pub(super) fn parse(arguments: &[String]) -> Result<Options, String> {
    let (mut context, mut report, mut log, mut faults) = (None, None, None, None);
    let (mut kubeconfig, mut compare) = (Vec::new(), Vec::new());
    let (mut minutes, mut memory_limit_mib, mut warm_up) = (60, 64, 10);
    let mut rest = arguments.iter();
    while let Some(option) = rest.next() {
        let mut value = || rest.next().ok_or_else(|| format!("the option {option} needs a value"));
        match option.as_str() {
            "--context" => context = Some(value()?.clone()),
            "--kubeconfig" => kubeconfig.push(PathBuf::from(value()?)),
            "--compare" => compare.push(PathBuf::from(value()?)),
            "--report" => report = Some(PathBuf::from(value()?)),
            "--minutes" => minutes = number(option, value()?, 1, MAX_MINUTES)?,
            "--log" => log = Some(deployment(value()?)?),
            "--memory-limit" => memory_limit_mib = number(option, value()?, 1, MAX_MEMORY_LIMIT)?,
            "--warm-up" => warm_up = number(option, value()?, 0, MAX_MINUTES)?,
            "--faults" => faults = Some(PathBuf::from(value()?)),
            other => return Err(format!("`{other}` is not an option of the command")),
        }
    }
    let context = context.ok_or("the soak needs --context")?;
    let report = report.ok_or("the soak needs --report")?;
    if kubeconfig.is_empty() {
        return Err("the soak needs --kubeconfig: it never reads the profile of the user".to_owned());
    }
    Ok(Options { context, kubeconfig, compare, report, minutes, log, memory_limit_mib, warm_up, faults })
}

fn number(option: &str, text: &str, min: u64, max: u64) -> Result<u64, String> {
    let number = text.parse().ok().filter(|number| (min..=max).contains(number));
    number.ok_or_else(|| format!("the value of {option} is a number from {min} to {max}, not `{text}`"))
}

fn deployment(text: &str) -> Result<(String, String), String> {
    match text.split_once('/') {
        Some((namespace, name)) if !namespace.is_empty() && !name.is_empty() => {
            Ok((namespace.to_owned(), name.to_owned()))
        }
        _ => Err(format!("the value of --log is <namespace>/<name>, not `{text}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Result<Options, String> {
        parse(&line.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
    }

    #[test]
    fn the_needed_options_and_each_default() {
        let options = parsed("--context k3s --kubeconfig /k/a --report /r").expect("good options");
        assert_eq!((options.context.as_str(), options.kubeconfig.len(), options.compare.len()), ("k3s", 1, 0));
        assert_eq!((options.minutes, options.memory_limit_mib, options.warm_up), (60, 64, 10));
        assert_eq!((options.log, options.faults), (None, None));

        let line = "--context k3s --kubeconfig /k/a --compare /k/b --report /r --minutes 5 --log soak/logger \
                    --memory-limit 32 --warm-up 2 --faults /r/faults";
        let options = parsed(line).expect("good options");
        assert_eq!((options.minutes, options.memory_limit_mib, options.warm_up), (5, 32, 2));
        assert_eq!(options.log, Some(("soak".to_owned(), "logger".to_owned())));
        assert_eq!((options.compare, options.faults), (vec![PathBuf::from("/k/b")], Some(PathBuf::from("/r/faults"))));
    }

    #[test]
    fn a_bad_or_missing_option_gives_its_reason() {
        let error = |line: &str| parsed(line).expect_err("bad options");
        assert_eq!(error("--kubeconfig /k --report /r"), "the soak needs --context");
        assert_eq!(
            error("--context a --report /r"),
            "the soak needs --kubeconfig: it never reads the profile of the user"
        );
        assert_eq!(error("--context a --kubeconfig /k"), "the soak needs --report");
        assert_eq!(error("--minutes 0"), "the value of --minutes is a number from 1 to 10080, not `0`");
        assert_eq!(error("--log logger"), "the value of --log is <namespace>/<name>, not `logger`");
        assert_eq!(error("--hours 1"), "`--hours` is not an option of the command");
    }
}
