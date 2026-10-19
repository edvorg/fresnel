//! `cargo xtask notices`: the license files of a release. The MIT and Apache licenses of the
//! crates in the program and the OFL of JetBrains Mono ask that their notices go with each copy
//! of the program, so `install` puts the files into `Contents/Resources` of the bundle and
//! `windows` next to `fresnel.exe`.
//!
//! - `crates`: the crates of others in the program, from cargo.
//! - `package`: the license files of the package of a crate.
//! - `document`: the text of `THIRD-PARTY-LICENSES`.

mod crates;
mod document;
mod package;

use std::fs;
use std::io;
use std::path::Path;

/// The names of the files: the licenses of fresnel and the one of the parts of others.
pub const FILES: [&str; 3] = [MIT, APACHE, THIRD_PARTY];
const MIT: &str = "LICENSE-MIT";
const APACHE: &str = "LICENSE-APACHE";
const THIRD_PARTY: &str = "THIRD-PARTY-LICENSES";
/// The license of the font of the terminal, which `fresnel-ui` embeds, with its copyright.
const FONT_LICENSE: &str = "crates/fresnel-ui/fonts/OFL.txt";

/// The license files of a release, ready to write.
pub struct Notices {
    mit: String,
    apache: String,
    third_party: String,
    crates: usize,
    texts: usize,
}

impl Notices {
    /// The files for the program of the workspace at `root`. A crate whose license text the
    /// files would miss stops it.
    pub fn of_workspace(root: &Path) -> io::Result<Self> {
        let read = |file: &str| {
            fs::read_to_string(root.join(file))
                .map_err(|error| io::Error::new(error.kind(), format!("{file}: {error}")))
        };
        let (mit, apache, font) = (read(MIT)?, read(APACHE)?, read(FONT_LICENSE)?);
        let crates: Vec<package::Crate> =
            crates::shipped(root)?.into_iter().map(package::read).collect::<io::Result<_>>()?;
        let document = document::write(&crates, &document::Ours { font: &font, mit: &mit, apache: &apache })?;
        Ok(Self { mit, apache, third_party: document.text, crates: crates.len(), texts: document.texts })
    }

    /// The text of `THIRD-PARTY-LICENSES`.
    pub fn third_party(&self) -> &str {
        &self.third_party
    }

    /// The count of crates of others in the program.
    pub fn crates(&self) -> usize {
        self.crates
    }

    /// The count of different license texts of those crates.
    pub fn texts(&self) -> usize {
        self.texts
    }

    /// Writes each file of [`FILES`] into `folder`, which exists. The licenses of fresnel keep
    /// the bytes of the repository.
    pub fn write_into(&self, folder: &Path) -> io::Result<()> {
        for (name, text) in [(MIT, &self.mit), (APACHE, &self.apache), (THIRD_PARTY, &self.third_party)] {
            fs::write(folder.join(name), text)?;
        }
        Ok(())
    }
}
