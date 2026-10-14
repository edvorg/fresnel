//! The bundle and its install, in the test folder of the build.
//!
//! No test writes to `/Applications`. The tests give the bundle this test program in
//! place of the app, so they need no release build.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::path::{Path, PathBuf};

use xtask::install::{Installed, bundle, swap};

/// A new folder for one test, below the test folder of the build. Its drop removes it.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{label}-{}", std::process::id()));
        if folder.exists() {
            std::fs::remove_dir_all(&folder).expect("the folder of an earlier run goes away");
        }
        std::fs::create_dir_all(&folder).expect("a new folder");
        Self(folder)
    }

    fn folder(&self, name: &str) -> PathBuf {
        let folder = self.0.join(name);
        std::fs::create_dir_all(&folder).expect("a new folder");
        folder
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}

/// A staged bundle whose contents hold only a mark, for the tests of the moves.
fn staged(scratch: &Scratch, mark: &str) -> PathBuf {
    let app = scratch.folder("stage").join(bundle::APP);
    std::fs::create_dir_all(app.join("Contents")).expect("a staged bundle");
    std::fs::write(app.join("Contents/mark"), mark).expect("the mark");
    app
}

fn mark(bundle: &Path) -> String {
    std::fs::read_to_string(bundle.join("Contents/mark")).expect("the mark of a bundle")
}

/// The identity of a folder in its file system: a link of macOS, such as an item of the
/// Dock, finds the folder by it.
fn identity(folder: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(folder).expect("the folder").ino()
}

fn names(folder: &Path) -> Vec<String> {
    let entries = std::fs::read_dir(folder).expect("the folder");
    let mut names: Vec<String> =
        entries.map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn an_install_keeps_the_bundle_before_it_as_the_only_fallback() {
    let scratch = Scratch::new("swap");
    let applications = scratch.folder("Applications");
    let fallback = swap::fallback_path(&applications);

    let first = swap::install(&staged(&scratch, "one"), &applications).expect("the first install");
    assert_eq!(first, Installed { app: applications.join("Fresnel.app"), fallback: None });
    assert_eq!(names(&applications), ["Fresnel.app"]);

    let app = identity(&first.app);
    let second = swap::install(&staged(&scratch, "two"), &applications).expect("the second install");
    assert_eq!(second.fallback.as_deref(), Some(fallback.as_path()));
    assert_eq!(names(&applications), ["Fresnel.app", "Fresnel.app.prev"]);
    assert_eq!((mark(&second.app), mark(&fallback)), ("two".to_owned(), "one".to_owned()));
    assert_eq!(identity(&second.app), app, "a link to the app finds the new version");
    assert_ne!(identity(&fallback), app, "a link to the app does not find the fallback");

    swap::install(&staged(&scratch, "three"), &applications).expect("the third install");
    assert_eq!(names(&applications), ["Fresnel.app", "Fresnel.app.prev"], "the older fallback went away");
    assert_eq!((mark(&applications.join("Fresnel.app")), mark(&fallback)), ("three".to_owned(), "two".to_owned()));
    assert_eq!(identity(&applications.join("Fresnel.app")), app);
    assert_eq!(names(&scratch.folder("stage")), Vec::<String>::new(), "each install moved its staged bundle");
}

/// An app folder with no contents, from an install that stopped between its two moves,
/// gets the new contents and keeps the fallback that it had.
#[test]
fn an_app_folder_with_no_contents_gets_the_new_contents() {
    let scratch = Scratch::new("swap-empty");
    let applications = scratch.folder("Applications");
    swap::install(&staged(&scratch, "one"), &applications).expect("the first install");
    swap::install(&staged(&scratch, "two"), &applications).expect("the second install");
    std::fs::remove_dir_all(applications.join("Fresnel.app/Contents")).expect("no contents");

    let installed = swap::install(&staged(&scratch, "three"), &applications).expect("the install");
    assert_eq!(installed.fallback, Some(swap::fallback_path(&applications)));
    assert_eq!(
        (mark(&installed.app), mark(&swap::fallback_path(&applications))),
        ("three".to_owned(), "one".to_owned())
    );
    assert_eq!(names(&scratch.folder("stage")), Vec::<String>::new());
}

#[test]
fn an_install_that_fails_leaves_the_folder_as_it_was() {
    let scratch = Scratch::new("swap-failure");
    let applications = scratch.folder("Applications");
    swap::install(&staged(&scratch, "one"), &applications).expect("the first install");
    swap::install(&staged(&scratch, "two"), &applications).expect("the second install");

    let absent = scratch.folder("stage").join("absent.app");
    let error = swap::install(&absent, &applications).expect_err("no staged bundle");
    assert!(error.to_string().starts_with("no bundle at "), "{error}");
    let file = scratch.folder("stage").join("a-file");
    std::fs::write(&file, "x").expect("a file");
    swap::install(&file, &applications).expect_err("a file is no bundle");
    assert_eq!(names(&applications), ["Fresnel.app", "Fresnel.app.prev"]);
    assert_eq!(mark(&applications.join("Fresnel.app")), "two");
    assert_eq!(mark(&swap::fallback_path(&applications)), "one");
}

#[test]
fn an_install_whose_last_step_fails_puts_each_bundle_back() {
    use std::os::unix::fs::PermissionsExt as _;

    let scratch = Scratch::new("swap-rollback");
    let applications = scratch.folder("Applications");
    swap::install(&staged(&scratch, "one"), &applications).expect("the first install");
    swap::install(&staged(&scratch, "two"), &applications).expect("the second install");

    // The staged contents cannot leave a bundle that takes no change. The step before it,
    // the move of the installed contents to the fallback, works.
    let three = staged(&scratch, "three");
    let permissions = |mode| std::fs::set_permissions(&three, std::fs::Permissions::from_mode(mode));
    permissions(0o555).expect("a folder that takes no change");
    let error = swap::install(&three, &applications);
    permissions(0o755).expect("the folder takes changes again");

    error.expect_err("the staged bundle did not move");
    assert_eq!(names(&applications), ["Fresnel.app", "Fresnel.app.prev"]);
    assert_eq!(mark(&applications.join("Fresnel.app")), "two");
    assert_eq!(mark(&swap::fallback_path(&applications)), "one", "the older fallback is back too");
    assert_eq!(mark(&three), "three");
}

#[test]
fn the_stage_folder_goes_away_and_an_error_of_the_install_comes_before_an_error_of_the_removal() {
    let scratch = Scratch::new("stage");
    let applications = scratch.0.join("Applications");
    let staged = xtask::install::with_stage(&applications, |stage| {
        std::fs::write(stage.join("a-file"), "x")?;
        Ok(stage.to_owned())
    });
    let stage = staged.expect("the work");
    assert_eq!((stage.parent(), stage.exists()), (Some(applications.as_path()), false));
    assert_eq!(names(&applications), Vec::<String>::new());

    // The work fails, and the removal fails too: the folder is gone.
    let failed = xtask::install::with_stage(&applications, |stage| {
        std::fs::remove_dir(stage)?;
        Err::<(), _>(std::io::Error::other("codesign failed"))
    });
    assert_eq!(failed.expect_err("the work failed").to_string(), "codesign failed");
    let unremoved = xtask::install::with_stage(&applications, |stage| std::fs::remove_dir(stage));
    assert_eq!(unremoved.expect_err("the removal failed").kind(), std::io::ErrorKind::NotFound);
}

/// The tools `iconutil`, `codesign` and `plutil` are tools of macOS.
#[cfg(target_os = "macos")]
mod macos {
    use std::os::unix::fs::PermissionsExt as _;
    use std::process::Command;

    use super::*;

    fn output(tool: &str, arguments: &[&str], path: &Path) -> (bool, String) {
        let output = Command::new(tool).args(arguments).arg(path).output().expect("the tool starts");
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        (output.status.success(), text)
    }

    #[test]
    fn the_bundle_has_the_program_the_plist_and_the_icon_and_its_signature_fits() {
        let scratch = Scratch::new("bundle");
        let program = std::env::current_exe().expect("this test program");
        let app = bundle::assemble(&program, "0.1.0", &scratch.folder("stage")).expect("the bundle");
        assert_eq!(names(&scratch.folder("stage")), ["Fresnel.app"], "the icon set is gone");
        assert_eq!(names(&app.join("Contents")), ["Info.plist", "MacOS", "PkgInfo", "Resources"]);

        let installed = app.join("Contents/MacOS/fresnel");
        let meta = installed.metadata().expect("the program of the bundle");
        assert_eq!(meta.len(), program.metadata().expect("this test program").len());
        assert_eq!(meta.permissions().mode() & 0o777, 0o755);
        let plist = app.join("Contents/Info.plist");
        let (valid, text) = output("plutil", &["-lint"], &plist);
        assert!(valid, "{text}");
        let (_, version) = output("plutil", &["-extract", "CFBundleShortVersionString", "raw"], &plist);
        assert_eq!(version.trim(), "0.1.0");

        // An icon file starts with its type and its length.
        let icon = std::fs::read(app.join("Contents/Resources/AppIcon.icns")).expect("the icon");
        assert_eq!(&icon[..4], b"icns");
        assert_eq!(u32::from_be_bytes([icon[4], icon[5], icon[6], icon[7]]) as usize, icon.len());
        assert!(icon.len() > 20_000, "the icon has {} bytes", icon.len());

        bundle::sign(&app).expect("the signature");
        let (signed, facts) = output("codesign", &["--display", "--verbose=2"], &app);
        assert!(signed && facts.contains("Signature=adhoc"), "{facts}");
        assert!(facts.contains("Identifier=io.github.edvorg.fresnel"), "{facts}");
        assert!(facts.contains("Info.plist entries=15"), "the signature covers the plist: {facts}");

        // The signature covers each file: a changed plist does not fit it.
        std::fs::write(&plist, bundle::info_plist("0.1.1")).expect("a changed plist");
        let error = bundle::verify(&app).expect_err("the signature does not fit");
        assert!(error.to_string().starts_with("codesign failed"), "{error}");
    }

    #[test]
    fn a_signed_bundle_installs_by_rename_and_its_signature_still_fits() {
        let scratch = Scratch::new("signed-install");
        let applications = scratch.folder("Applications");
        let program = std::env::current_exe().expect("this test program");
        for version in ["0.1.0", "0.1.1"] {
            let app = bundle::assemble(&program, version, &scratch.folder("stage")).expect("the bundle");
            bundle::sign(&app).expect("the signature");
            swap::install(&app, &applications).expect("the install");
        }
        for (bundle_path, version) in
            [(applications.join("Fresnel.app"), "0.1.1"), (swap::fallback_path(&applications), "0.1.0")]
        {
            bundle::verify(&bundle_path).expect("the signature of an installed bundle fits");
            let plist = bundle_path.join("Contents/Info.plist");
            let (_, found) = output("plutil", &["-extract", "CFBundleShortVersionString", "raw"], &plist);
            assert_eq!(found.trim(), version);
        }
    }
}
