//! The bundle `Fresnel.app`: the program, `Info.plist` and the icon.

use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use super::icon;
use super::tool::run;

/// The name of the bundle folder.
pub const APP: &str = "Fresnel.app";
/// The identifier of the bundle. macOS knows the app by it.
pub const IDENTIFIER: &str = "io.github.edvorg.fresnel";
const PROGRAM: &str = "fresnel";
const ICON: &str = "AppIcon";
/// The oldest macOS that the Rust toolchain builds for on Apple silicon.
const OLDEST_SYSTEM: &str = "11.0";

/// Makes the bundle in `folder` from a built program. Returns the path of the bundle.
/// The icon needs `iconutil`, a tool of macOS.
pub fn assemble(program: &Path, version: &str, folder: &Path) -> io::Result<PathBuf> {
    let app = folder.join(APP);
    let contents = app.join("Contents");
    let (programs, resources) = (contents.join("MacOS"), contents.join("Resources"));
    std::fs::create_dir_all(&programs)?;
    std::fs::create_dir_all(&resources)?;

    let installed = programs.join(PROGRAM);
    std::fs::copy(program, &installed)?;
    std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755))?;
    std::fs::write(contents.join("Info.plist"), info_plist(version))?;
    std::fs::write(contents.join("PkgInfo"), "APPL????")?;

    let icon_set = folder.join(format!("{ICON}.iconset"));
    std::fs::create_dir_all(&icon_set)?;
    icon::write_icon_set(&icon_set)?;
    let icns = resources.join(format!("{ICON}.icns"));
    run("iconutil", [Path::new("--convert"), Path::new("icns"), Path::new("--output"), &icns, &icon_set])?;
    std::fs::remove_dir_all(&icon_set)?;
    Ok(app)
}

/// The text of `Info.plist` for a version of the app.
pub fn info_plist(version: &str) -> String {
    let entries = [
        ("CFBundleDevelopmentRegion", "en"),
        ("CFBundleDisplayName", "Fresnel"),
        ("CFBundleExecutable", PROGRAM),
        ("CFBundleIconFile", ICON),
        ("CFBundleIdentifier", IDENTIFIER),
        ("CFBundleInfoDictionaryVersion", "6.0"),
        ("CFBundleName", "Fresnel"),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleShortVersionString", version),
        ("CFBundleVersion", version),
        ("LSApplicationCategoryType", "public.app-category.developer-tools"),
        ("LSMinimumSystemVersion", OLDEST_SYSTEM),
        ("NSPrincipalClass", "NSApplication"),
    ];
    // A Retina display draws the window at its full resolution. A Mac with two GPUs can
    // use the one that needs less power.
    let switches = ["NSHighResolutionCapable", "NSSupportsAutomaticGraphicsSwitching"];
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    for (key, value) in entries {
        text.push_str(&format!("\t<key>{key}</key>\n\t<string>{}</string>\n", escape(value)));
    }
    for key in switches {
        text.push_str(&format!("\t<key>{key}</key>\n\t<true/>\n"));
    }
    text.push_str("</dict>\n</plist>\n");
    text
}

/// A text as the content of an XML element.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Signs the bundle with an ad hoc signature and checks the result. macOS on Apple
/// silicon starts only signed code, and a bundle needs a signature over all its files.
pub fn sign(app: &Path) -> io::Result<()> {
    run("codesign", [Path::new("--force"), Path::new("--sign"), Path::new("-"), app])?;
    verify(app)
}

/// Checks that the signature of the bundle fits each of its files.
pub fn verify(app: &Path) -> io::Result<()> {
    run("codesign", [Path::new("--verify"), Path::new("--deep"), Path::new("--strict"), app])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plist_has_the_version_the_program_and_the_identifier() {
        let plist = info_plist("1.2.3 <test & more>");
        for entry in [
            "\t<key>CFBundleExecutable</key>\n\t<string>fresnel</string>\n",
            "\t<key>CFBundleIdentifier</key>\n\t<string>io.github.edvorg.fresnel</string>\n",
            "\t<key>CFBundleShortVersionString</key>\n\t<string>1.2.3 &lt;test &amp; more&gt;</string>\n",
            "\t<key>CFBundleIconFile</key>\n\t<string>AppIcon</string>\n",
            "\t<key>NSHighResolutionCapable</key>\n\t<true/>\n",
        ] {
            assert!(plist.contains(entry), "{entry} in {plist}");
        }
        assert!(plist.starts_with("<?xml version=\"1.0\"") && plist.ends_with("</dict>\n</plist>\n"), "{plist}");
        assert_eq!(plist.matches("<key>").count(), 15);
    }
}
