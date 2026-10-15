//! The environment and the program files of Windows.

use std::ffi::OsStr;

use super::*;

fn env(pairs: &[(&str, &str)]) -> ProcessEnv {
    pairs.iter().copied().collect()
}

/// A folder of the test with the named files. Its drop removes it.
struct Folder(PathBuf);

impl Folder {
    fn with(files: &[&str]) -> Self {
        let folder = std::env::temp_dir().join(format!("fresnel-net-test-programs-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder).expect("a new folder");
        files.iter().for_each(|file| std::fs::write(folder.join(file), "").expect("a file"));
        Self(folder)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("the folder goes away");
    }
}

#[test]
fn a_variable_has_one_value_for_each_case_of_its_name() {
    let mut vars = env(&[("Path", r"C:\Windows"), ("UserProfile", r"C:\Users\tester")]);
    assert_eq!(vars.get("PATH"), Some(OsStr::new(r"C:\Windows")));
    vars.set("path", r"C:\Tools");
    assert_eq!((vars.len(), vars.get("Path")), (2, Some(OsStr::new(r"C:\Tools"))));
    assert_eq!(vars.home(), Some(OsStr::new(r"C:\Users\tester")), "the home folder is USERPROFILE");
    assert_eq!(env(&[("HOME", r"C:\msys")]).home(), None, "HOME is not the home folder on Windows");
}

#[test]
fn a_folder_of_path_with_a_quote_does_not_go_into_path() {
    let mut vars = env(&[("PATH", r"C:\Windows;C:\Tools")]);
    let folders = |names: &[&str]| names.iter().map(PathBuf::from).collect::<Vec<_>>();
    vars.extend_path(&folders(&[r"C:\First", r"C:\Tools"]), &folders(&[r"C:\Last", "C:\\bad\"name"]));
    assert_eq!(vars.get("PATH"), Some(OsStr::new(r"C:\First;C:\Windows;C:\Tools;C:\Last")));
}

/// `doctl` names `doctl.exe`, as kubectl finds it; a name with an extension is tried as it is
/// first, and a file with no extension is not a program.
#[test]
fn a_bare_name_finds_the_file_with_an_extension_of_pathext() {
    let folder = Folder::with(&["doctl.exe", "tool.cmd", "script", "kubectl-1.30"]);
    let path = folder.0.as_os_str().to_str().expect("a text path");
    let vars = env(&[("PATH", &format!(r"C:\nonexistent;{path}")), ("PATHEXT", ".COM;.EXE;.BAT;.CMD")]);
    assert_eq!(vars.find_program("doctl", None), Some(folder.0.join("doctl.EXE")));
    assert_eq!(vars.find_program("doctl.exe", None), Some(folder.0.join("doctl.exe")));
    assert_eq!(vars.find_program("tool", None), Some(folder.0.join("tool.CMD")));
    assert_eq!(vars.find_program("script", None), None, "no extension of PATHEXT");
    assert_eq!(vars.find_program("kubectl-1.30", None), Some(folder.0.join("kubectl-1.30")));
    let relative = env(&[]).find_program(r".\doctl", Some(&folder.0));
    assert_eq!(relative, Some(folder.0.join(r".\doctl.EXE")), "a relative path starts from the folder");
    assert_eq!(env(&[]).find_program(&format!(r"{path}\doctl"), None), Some(folder.0.join("doctl.EXE")));
}
