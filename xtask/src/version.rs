//! The version of a build of the program: the version of the workspace and, in the release
//! workflow, the number of its run, as in `0.1.0+build.7`.
//!
//! `cargo xtask` reads the number from `FRESNEL_BUILD` and gives the text to the build of the
//! program (`FRESNEL_VERSION`, which `fresnel --version` prints), to `Info.plist` and to the
//! resource file of Windows. One rule thus makes each form, and the program needs no build script.

use std::fmt::{self, Display};

/// The variable with the build number: the run number of the release workflow.
pub const BUILD_VARIABLE: &str = "FRESNEL_BUILD";
/// The variable that gives the text of the version to the build of the program.
pub const VERSION_VARIABLE: &str = "FRESNEL_VERSION";
/// The version of the workspace, which each crate has.
const WORKSPACE: &str = env!("CARGO_PKG_VERSION");

/// A version: three numbers and, for a build of the release workflow, a build number. Each
/// number has 16 bits, as in the version resource of Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    /// The major, minor and patch number of the workspace.
    pub release: [u16; 3],
    /// The build number. None for a build outside the release workflow.
    pub build: Option<u16>,
}

impl Version {
    /// The version of this build: the workspace and the variable `FRESNEL_BUILD` of the process.
    pub fn of_environment() -> Result<Self, String> {
        let build = std::env::var_os(BUILD_VARIABLE).map(|value| value.to_string_lossy().into_owned());
        Self::parse(WORKSPACE, build.as_deref())
    }

    /// The version of a workspace version and a build number. A number that does not fit is an
    /// error: a release with a part of its version lost would look like an other release.
    pub fn parse(workspace: &str, build: Option<&str>) -> Result<Self, String> {
        let parts: Vec<Option<u16>> = workspace.split('.').map(number).collect();
        let release = match parts.as_slice() {
            [Some(major), Some(minor), Some(patch)] => [*major, *minor, *patch],
            _ => return Err(format!("the workspace version {workspace} is not three numbers of 0 to 65535")),
        };
        let build = match build {
            None => None,
            Some(text) => Some(number(text).ok_or_else(|| {
                format!("{BUILD_VARIABLE} is {text:?}, not a number of 0 to 65535 for the version resource of Windows")
            })?),
        };
        Ok(Self { release, build })
    }

    /// The text of the workspace version, as `0.1.0`.
    pub fn release_text(&self) -> String {
        let [major, minor, patch] = self.release;
        format!("{major}.{minor}.{patch}")
    }

    /// The four numbers of the version resource of Windows. The last is the build number, 0
    /// for a build outside the release workflow.
    pub fn numbers(&self) -> [u16; 4] {
        let [major, minor, patch] = self.release;
        [major, minor, patch, self.build.unwrap_or(0)]
    }
}

/// The full text: `0.1.0`, or `0.1.0+build.7` with a build number (build metadata of semver).
impl Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.release_text())?;
        match self.build {
            Some(build) => write!(f, "+build.{build}"),
            None => Ok(()),
        }
    }
}

/// A number of decimal digits only: `u16::from_str` also takes a sign.
fn number(text: &str) -> Option<u16> {
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if digits { text.parse().ok() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_number_gives_the_build_metadata_and_the_fourth_number() {
        let version = Version::parse("0.1.0", Some("57")).expect("a version");
        assert_eq!(version, Version { release: [0, 1, 0], build: Some(57) });
        assert_eq!((version.to_string(), version.release_text()), ("0.1.0+build.57".into(), "0.1.0".into()));
        assert_eq!(version.numbers(), [0, 1, 0, 57]);

        let local = Version::parse("2.10.3", None).expect("a version");
        assert_eq!((local.to_string(), local.numbers()), ("2.10.3".into(), [2, 10, 3, 0]));
    }

    #[test]
    fn a_number_that_does_not_fit_is_an_error() {
        for build in ["", "x", "+7", "-1", "65536", "1.2", " 7"] {
            let error = Version::parse("0.1.0", Some(build)).expect_err(build);
            assert!(error.starts_with("FRESNEL_BUILD is"), "{error}");
        }
        assert_eq!(Version::parse("0.1.0", Some("65535")).map(|version| version.build), Ok(Some(65535)));
        for workspace in ["0.1", "0.1.0.0", "0.1.0-alpha", "0.70000.0", ""] {
            let error = Version::parse(workspace, None).expect_err(workspace);
            assert!(error.starts_with("the workspace version"), "{error}");
        }
    }
}
