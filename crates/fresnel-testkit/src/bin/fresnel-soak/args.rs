//! The options of the harness.

use std::path::PathBuf;

pub(crate) const USAGE: &str = "\
usage: fresnel-soak --minutes <number> --report <folder> [options]

Starts k3s in Docker behind a fault proxy, changes objects all the time, makes random faults,
and runs `fresnel-cli soak` on the cluster. `cargo xtask soak` builds and starts it.

options:
  --minutes <number>  the run time
  --report <folder>   the folder of the report
  --label <text>      a part of the name of the containers (default soak)
  --seed <number>     the seed of the random changes and faults (default: from the clock)
  --fast              four times the changes and the faults, ten times the log lines, and
                      one recreate of the cluster in the middle of the run: each object
                      gets a new UID. A restart of the API server comes each 20 minutes
                      at most
  --cli <file>        the program fresnel-cli (default: the one next to this program)";

pub(crate) struct Options {
    pub(crate) minutes: u64,
    pub(crate) report: PathBuf,
    pub(crate) label: String,
    pub(crate) seed: u64,
    pub(crate) fast: bool,
    pub(crate) cli: PathBuf,
}

pub(crate) fn parse(arguments: &[String]) -> Result<Options, String> {
    let (mut minutes, mut report, mut label, mut seed, mut cli) = (None, None, "soak".to_owned(), None, None);
    let mut fast = false;
    let mut rest = arguments.iter();
    while let Some(option) = rest.next() {
        if option == "--fast" {
            fast = true;
            continue;
        }
        let mut value = || rest.next().ok_or_else(|| format!("the option {option} needs a value"));
        match option.as_str() {
            "--minutes" => minutes = Some(number(option, value()?)?),
            "--report" => report = Some(PathBuf::from(value()?)),
            "--label" => label = value()?.clone(),
            "--seed" => seed = Some(number(option, value()?)?),
            "--cli" => cli = Some(PathBuf::from(value()?)),
            other => return Err(format!("`{other}` is not an option")),
        }
    }
    let minutes = minutes.filter(|minutes| *minutes > 0).ok_or("the soak needs --minutes")?;
    let report = report.ok_or("the soak needs --report")?;
    let cli = match cli {
        Some(cli) => cli,
        None => next_to_this_program()?,
    };
    Ok(Options { minutes, report, label, seed: seed.unwrap_or_else(|| fastrand::u64(..)), fast, cli })
}

fn number(option: &str, text: &str) -> Result<u64, String> {
    text.parse().map_err(|_| format!("the value of {option} is a number, not `{text}`"))
}

/// Cargo builds `fresnel-cli` into the same folder as this program.
fn next_to_this_program() -> Result<PathBuf, String> {
    let program = std::env::current_exe().map_err(|error| format!("the path of this program: {error}"))?;
    let cli = program.with_file_name("fresnel-cli");
    if cli.is_file() { Ok(cli) } else { Err(format!("no program {}; build fresnel-cli first", cli.display())) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(arguments: &[&str]) -> Result<Options, String> {
        parse(&arguments.iter().map(|argument| (*argument).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn the_options_take_their_values_and_the_defaults_fill_the_rest() {
        let options = parsed(&["--minutes", "90", "--report", "out", "--seed", "7", "--fast", "--cli", "bin/cli"])
            .unwrap_or_else(|problem| panic!("{problem}"));
        let values = (options.minutes, options.report, options.label, options.seed, options.fast, options.cli);
        assert_eq!(values, (90, "out".into(), "soak".to_owned(), 7, true, "bin/cli".into()));
        let options = parsed(&["--label", "night", "--minutes", "1", "--report", "r", "--cli", "c"]);
        assert!(options.is_ok_and(|options| options.label == "night" && !options.fast));
    }

    /// A soak of no time or with no report would end with nothing to look at.
    #[test]
    fn a_missing_or_bad_value_is_a_problem_with_its_option() {
        let problem = |arguments: &[&str]| parsed(arguments).err().unwrap_or_default();
        assert_eq!(problem(&["--report", "r", "--cli", "c"]), "the soak needs --minutes");
        assert_eq!(problem(&["--minutes", "0", "--report", "r", "--cli", "c"]), "the soak needs --minutes");
        assert_eq!(problem(&["--minutes", "5", "--cli", "c"]), "the soak needs --report");
        assert_eq!(problem(&["--minutes", "an hour"]), "the value of --minutes is a number, not `an hour`");
        assert_eq!(problem(&["--minutes"]), "the option --minutes needs a value");
        assert_eq!(problem(&["--hours", "1"]), "`--hours` is not an option");
    }
}
