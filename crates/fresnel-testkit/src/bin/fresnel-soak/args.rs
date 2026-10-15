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
  --cli <file>        the program fresnel-cli (default: the one next to this program)";

pub(crate) struct Options {
    pub(crate) minutes: u64,
    pub(crate) report: PathBuf,
    pub(crate) label: String,
    pub(crate) seed: u64,
    pub(crate) cli: PathBuf,
}

pub(crate) fn parse(arguments: &[String]) -> Result<Options, String> {
    let (mut minutes, mut report, mut label, mut seed, mut cli) = (None, None, "soak".to_owned(), None, None);
    let mut rest = arguments.iter();
    while let Some(option) = rest.next() {
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
    Ok(Options { minutes, report, label, seed: seed.unwrap_or_else(|| fastrand::u64(..)), cli })
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
