//! `cargo xtask soak --hours <N>` (plan section 13, line 8): builds `fresnel-cli` and the
//! harness `fresnel-soak` of the test kit in a release build, and runs the harness: k3s in
//! Docker behind the fault proxy, objects that change, random faults, and `fresnel-cli soak`
//! on the cluster. This crate depends on no other crate of the workspace, so the harness is
//! a program of its own.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

/// The longest soak: one week.
const MAX_HOURS: f64 = 168.0;

/// The options of the command.
#[derive(Debug, PartialEq)]
pub struct Soak {
    pub minutes: u64,
    /// A part of the names of the containers, so that `docker ps` tells whose they are.
    pub label: Option<String>,
    /// The folder of the report. `None`: a new folder below the target folder.
    pub report: Option<PathBuf>,
    pub seed: Option<u64>,
}

/// Reads the options after `soak`. The error is a text for the user.
pub fn parse(arguments: &[&str]) -> Result<Soak, String> {
    let mut soak = Soak { minutes: 0, label: None, report: None, seed: None };
    let mut rest = arguments.iter();
    while let Some(option) = rest.next() {
        let value = rest.next().ok_or_else(|| format!("the option {option} needs a value"))?;
        match *option {
            "--hours" => soak.minutes = minutes(value)?,
            "--label" => soak.label = Some((*value).to_owned()),
            "--report" => soak.report = Some(PathBuf::from(value)),
            "--seed" => soak.seed = Some(value.parse().map_err(|_| format!("the seed is a number, not `{value}`"))?),
            other => return Err(format!("`{other}` is not an option of soak")),
        }
    }
    if soak.minutes == 0 {
        return Err("soak needs --hours".to_owned());
    }
    Ok(soak)
}

/// The minutes of a count of hours, which can have a fraction: `0.25` is 15 minutes.
fn minutes(text: &str) -> Result<u64, String> {
    let hours: f64 = text.parse().map_err(|_| format!("the hours are a number, not `{text}`"))?;
    if !(hours > 0.0 && hours <= MAX_HOURS) {
        return Err(format!("the hours are more than 0 and {MAX_HOURS} at most, not `{text}`"));
    }
    // The bounds above keep the value small and positive.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a positive value below 10,081")]
    Ok(((hours * 60.0).round() as u64).max(1))
}

/// Builds the two programs and runs the soak. Gives the exit status of the harness.
pub fn run(root: &Path, soak: &Soak) -> Result<ExitStatus, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let build =
        ["build", "--release", "--locked", "--package", "fresnel-cli", "--package", "fresnel-testkit", "--bins"];
    let built = Command::new(cargo).args(build).current_dir(root).status();
    let built = built.map_err(|error| format!("cannot start cargo: {error}"))?;
    if !built.success() {
        return Err(format!("the release build failed ({built})"));
    }
    // A relative folder of the variable starts from the root, as for the build above.
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from);
    let programs = root.join(target).join("release");
    let report = soak.report.clone().unwrap_or_else(|| programs.join("soak").join(seconds().to_string()));
    println!("soak: the report goes to {}", report.display());
    let mut harness = Command::new(programs.join("fresnel-soak"));
    harness.arg("--minutes").arg(soak.minutes.to_string()).arg("--report").arg(&report);
    harness.arg("--cli").arg(programs.join("fresnel-cli"));
    if let Some(label) = &soak.label {
        harness.args(["--label", label]);
    }
    if let Some(seed) = soak.seed {
        harness.args(["--seed", &seed.to_string()]);
    }
    harness.status().map_err(|error| format!("cannot start the harness: {error}"))
}

fn seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hours_can_have_a_fraction_and_each_other_option_is_optional() {
        let soak = parse(&["--hours", "0.25"]).expect("good options");
        assert_eq!(soak, Soak { minutes: 15, label: None, report: None, seed: None });
        let soak =
            parse(&["--hours", "24", "--label", "m8-soak", "--report", "/r", "--seed", "7"]).expect("good options");
        assert_eq!(
            soak,
            Soak { minutes: 1440, label: Some("m8-soak".to_owned()), report: Some(PathBuf::from("/r")), seed: Some(7) }
        );
    }

    #[test]
    fn a_bad_option_gives_its_reason() {
        let error = |arguments: &[&str]| parse(arguments).expect_err("bad options");
        assert_eq!(error(&[]), "soak needs --hours");
        assert_eq!(error(&["--hours"]), "the option --hours needs a value");
        assert_eq!(error(&["--hours", "0"]), "the hours are more than 0 and 168 at most, not `0`");
        assert_eq!(error(&["--hours", "a day"]), "the hours are a number, not `a day`");
        assert_eq!(error(&["--minutes", "5"]), "`--minutes` is not an option of soak");
    }
}
