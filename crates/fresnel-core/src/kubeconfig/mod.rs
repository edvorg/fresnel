//! The kubeconfig files of the user: each context of each file.
//!
//! The merge rule is the rule of `kubectl`: for each name, the first file wins. A file that
//! does not load is a problem in the result. It does not hide the other files.
//!
//! - `sources`: where the files are. `file`: the format of one file.
//! - `access`: one context as a checked way to its cluster. `material`: certificates and keys.

mod access;
mod file;
mod material;
mod sources;

use std::path::{Path, PathBuf};

pub(crate) use access::Identity;
pub use access::{Access, AuthKind, Server};
#[cfg(test)]
pub(crate) use material::Material;
pub use sources::default_sources;

use crate::error::CoreError;
use access::Entry;
use file::{ClusterEntry, ContextEntry, Named, UserEntry};

/// One context of the kubeconfig files: one cluster of the app.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    /// The name of the context. It is the identity of the cluster in the app.
    pub name: String,
    /// The namespace of the context, if it has one.
    pub namespace: Option<String>,
    /// The file that defines the context.
    pub source: PathBuf,
    /// How the app reaches the cluster. An error when the entries of the context are not
    /// complete, or when the app does not support them.
    pub access: Result<Access, CoreError>,
}

/// Something in a file that the load did not use. The text holds no content of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The file.
    pub path: PathBuf,
    /// What the problem is.
    pub text: String,
}

/// The result of one load of the kubeconfig files.
#[derive(Clone, Debug, Default)]
pub struct Kubeconfig {
    /// The files, in the sequence of their priority.
    pub sources: Vec<PathBuf>,
    /// The files that do not exist. `kubectl` skips such a file with no message.
    pub absent: Vec<PathBuf>,
    /// The files that exist and did not load. `problems` tells why. A caller that loads
    /// again can keep what such a file gave before.
    pub unread: Vec<PathBuf>,
    /// Each context of each file, in the sequence of the files.
    pub contexts: Vec<Context>,
    /// The `current-context` of the first file that has one.
    pub current: Option<String>,
    /// Each problem of each file.
    pub problems: Vec<Problem>,
}

/// Reads the files. The call reads from the disk, so it can block: call it off the UI thread.
pub fn load(sources: &[PathBuf]) -> Kubeconfig {
    let mut merge = Merge::default();
    for path in sources {
        merge.add(path);
    }
    let contexts = merge.contexts.iter().map(|context| merge.context(context)).collect();
    Kubeconfig {
        sources: sources.to_vec(),
        absent: merge.absent,
        unread: merge.unread,
        contexts,
        current: merge.current,
        problems: merge.problems,
    }
}

/// One named entry with the file that defines it. An entry with no body keeps its place, so
/// that a later file cannot define the name.
struct Located<T> {
    name: String,
    body: Option<T>,
    source: PathBuf,
}

/// The entries of all files, with the first entry of each name.
#[derive(Default)]
struct Merge {
    clusters: Vec<Located<ClusterEntry>>,
    users: Vec<Located<UserEntry>>,
    contexts: Vec<Located<ContextEntry>>,
    current: Option<String>,
    absent: Vec<PathBuf>,
    unread: Vec<PathBuf>,
    problems: Vec<Problem>,
}

impl Merge {
    fn add(&mut self, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return self.absent.push(path.to_owned()),
            Err(error) => return self.unread(path, format!("cannot read the file: {error}")),
        };
        let documents = match file::parse(&text) {
            Ok(documents) => documents,
            Err(at) => {
                let text = format!("the file is not a kubeconfig: line {}, column {}", at.line, at.column);
                return self.unread(path, text);
            }
        };
        for document in documents {
            keep(&mut self.clusters, &mut self.problems, "cluster", document.clusters, path);
            keep(&mut self.users, &mut self.problems, "user", document.users, path);
            keep(&mut self.contexts, &mut self.problems, "context", document.contexts, path);
            self.current = self.current.take().or(document.current_context.filter(|name| !name.is_empty()));
        }
    }

    /// A file that exists and gives no entry.
    fn unread(&mut self, path: &Path, text: String) {
        self.unread.push(path.to_owned());
        self.problems.push(Problem { path: path.to_owned(), text });
    }

    fn context(&self, located: &Located<ContextEntry>) -> Context {
        let access = match &located.body {
            Some(body) => self.access(body),
            None => Err(CoreError::invalid("the context entry has no body")),
        };
        let namespace = located.body.as_ref().and_then(|body| body.namespace.clone());
        Context {
            name: located.name.clone(),
            namespace: namespace.filter(|namespace| !namespace.is_empty()),
            source: located.source.clone(),
            access,
        }
    }

    fn access(&self, context: &ContextEntry) -> Result<Access, CoreError> {
        let cluster = entry(&self.clusters, "cluster", context.cluster.as_deref().unwrap_or_default())?;
        // `kubectl` connects with no credential when no file defines the user. That hides a
        // name with an error, so the app refuses it.
        let user = context.user.as_deref().filter(|user| !user.is_empty());
        let user = user.map(|user| entry(&self.users, "user", user)).transpose()?;
        Access::new(&cluster, user.as_ref())
    }
}

/// Adds the entries of one file. An entry with a name that the list has is a problem: a
/// context of this file that uses the name gets the entry of the earlier file.
fn keep<T>(
    list: &mut Vec<Located<T>>,
    problems: &mut Vec<Problem>,
    kind: &str,
    entries: Option<Vec<Named<T>>>,
    path: &Path,
) {
    for Named { name, body } in entries.into_iter().flatten() {
        if list.iter().any(|kept| kept.name == name) {
            let text = format!("the {kind} `{name}` has an earlier entry, which wins");
            problems.push(Problem { path: path.to_owned(), text });
        } else {
            list.push(Located { name, body, source: path.to_owned() });
        }
    }
}

fn entry<'a, T>(list: &'a [Located<T>], kind: &str, name: &'a str) -> Result<Entry<'a, T>, CoreError> {
    let located = list.iter().find(|located| located.name == name);
    let located = located.ok_or_else(|| CoreError::invalid(format!("no file defines the {kind} `{name}`")))?;
    let body = located.body.as_ref();
    let body = body.ok_or_else(|| CoreError::invalid(format!("the {kind} entry `{name}` has no body")))?;
    Ok(Entry { name, body, folder: located.source.parent().unwrap_or(Path::new("")) })
}

#[cfg(test)]
mod tests;
