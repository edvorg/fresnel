//! The contexts that a command works on: the options that select them, and their list.
//!
//! A command with no selected context lists the contexts and connects to nothing. A
//! connect runs the credential plugin of the user, so it needs an order.

use std::path::PathBuf;

use fresnel_core::kubeconfig::{AuthKind, Context, Kubeconfig};

use crate::words::size;

/// Which contexts a command works on.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Selection {
    /// None: the command lists the contexts.
    List,
    All,
    Names(Vec<String>),
}

/// The options `--context`, `--all` and `--kubeconfig` of a command line.
#[derive(Debug, Default)]
pub(crate) struct Picker {
    kubeconfig: Vec<PathBuf>,
    names: Vec<String>,
    all: bool,
}

impl Picker {
    pub(crate) fn all(&mut self) {
        self.all = true;
    }

    pub(crate) fn context(&mut self, name: &str) {
        self.names.push(name.to_owned());
    }

    pub(crate) fn kubeconfig(&mut self, file: &str) {
        self.kubeconfig.push(PathBuf::from(file));
    }

    /// The kubeconfig files and the selection. With no file, the command reads the
    /// kubeconfig files of the user. The error is a text for the user.
    pub(crate) fn finish(self) -> Result<(Vec<PathBuf>, Selection), String> {
        let selection = match (self.all, self.names.is_empty()) {
            (true, false) => return Err("use --all or --context, not the two".to_owned()),
            (true, true) => Selection::All,
            (false, false) => Selection::Names(self.names),
            (false, true) => Selection::List,
        };
        Ok((self.kubeconfig, selection))
    }
}

/// The selected contexts, in the sequence of the selection. `None`: the command lists the
/// contexts. The error is a name with no context.
pub(crate) fn selected<'a>(
    kubeconfig: &'a Kubeconfig,
    selection: &'a Selection,
) -> Result<Option<Vec<&'a Context>>, &'a str> {
    let contexts = &kubeconfig.contexts;
    let find = |name: &'a String| contexts.iter().find(|context| &context.name == name).ok_or(name.as_str());
    match selection {
        Selection::List => Ok(None),
        Selection::All => Ok(Some(contexts.iter().collect())),
        Selection::Names(names) => names.iter().map(find).collect::<Result<_, _>>().map(Some),
    }
}

/// The text of the credential of a context.
pub(crate) fn auth(kind: &AuthKind) -> String {
    match kind {
        AuthKind::Plugin(command) => format!("plugin {command}"),
        AuthKind::Certificate => "client certificate".to_owned(),
        AuthKind::Anonymous => "no credential".to_owned(),
    }
}

/// The list of the contexts, for a run with no selected context.
pub(crate) fn list(kubeconfig: &Kubeconfig) -> String {
    let files = size(kubeconfig.sources.len() - kubeconfig.absent.len(), "kubeconfig file");
    let mut lines = vec![format!("{} in {files}:", size(kubeconfig.contexts.len(), "context"))];
    for context in &kubeconfig.contexts {
        lines.push(match &context.access {
            Ok(access) => format!("  {}  {}  {}", context.name, access.server, auth(&access.auth())),
            Err(error) => format!("  {}  not usable: {error}", context.name),
        });
    }
    let problems = kubeconfig.problems.iter();
    lines.extend(problems.map(|problem| format!("problem: {}: {}", problem.path.display(), problem.text)));
    lines.push("Select a context with --context <name>, or each context with --all.".to_owned());
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use fresnel_core::kubeconfig;

    use super::*;

    fn picked(fill: impl FnOnce(&mut Picker)) -> Result<(Vec<PathBuf>, Selection), String> {
        let mut picker = Picker::default();
        fill(&mut picker);
        picker.finish()
    }

    #[test]
    fn no_option_lists_the_contexts_and_the_two_ways_to_select_exclude_each_other() {
        assert_eq!(picked(|_| {}), Ok((Vec::new(), Selection::List)));
        assert_eq!(picked(Picker::all), Ok((Vec::new(), Selection::All)));
        let named = picked(|picker| {
            picker.context("a");
            picker.kubeconfig("/k/one");
            picker.context("b");
        });
        assert_eq!(named, Ok((vec![PathBuf::from("/k/one")], Selection::Names(vec!["a".to_owned(), "b".to_owned()]))));
        let two = picked(|picker| {
            picker.all();
            picker.context("a");
        });
        assert_eq!(two, Err("use --all or --context, not the two".to_owned()));
    }

    #[test]
    fn the_selected_contexts_come_in_the_sequence_of_their_names() {
        let path = std::env::temp_dir().join(format!("fresnel-cli-test-named-{}.yaml", std::process::id()));
        std::fs::write(&path, "contexts:\n- name: a\n- name: b\n- name: c\n").expect("a file of the test");
        let loaded = kubeconfig::load(std::slice::from_ref(&path));
        std::fs::remove_file(&path).expect("the file goes");
        let names = |selection: &Selection| {
            let selected = selected(&loaded, selection).map_err(str::to_owned)?;
            Ok::<_, String>(
                selected.map(|contexts| contexts.iter().map(|context| context.name.clone()).collect::<Vec<_>>()),
            )
        };
        let pick = |names: &[&str]| Selection::Names(names.iter().map(|name| (*name).to_owned()).collect());
        assert_eq!(names(&pick(&["c", "a"])), Ok(Some(vec!["c".to_owned(), "a".to_owned()])));
        assert_eq!(names(&pick(&["a", "x"])), Err("x".to_owned()));
        assert_eq!(names(&Selection::All).map(|all| all.map(|all| all.len())), Ok(Some(3)));
        assert_eq!(names(&Selection::List), Ok(None));
        assert!(list(&loaded).starts_with("3 contexts in 1 kubeconfig file:\n  a  not usable: "), "{}", list(&loaded));
    }
}
