//! The filters of an object page, and what the worker collects of all objects for them.

use std::collections::BTreeSet;
use std::sync::Arc;

use fresnel_core::kinds::ObjectHealth;
use fresnel_core::store::ObjectKey;

/// The filters of the table of one kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObjectFilter {
    /// A part of a text of a row: the name, the namespace, the cluster or a text column.
    pub(crate) text: String,
    /// The namespaces whose objects show. Empty: each namespace. The namespace filter of
    /// the top bar sets it for each page.
    pub(crate) namespaces: BTreeSet<Arc<str>>,
    /// True to show only the objects that are not healthy.
    pub(crate) problems: bool,
}

impl ObjectFilter {
    /// The text filter as the rows match it: in lower case, with no space at its ends.
    pub(crate) fn needle(&self) -> String {
        self.text.trim().to_ascii_lowercase()
    }

    /// True when the namespace filter and the problems filter let the object pass. An
    /// object with no namespace passes each namespace filter. The text filter is the work
    /// of the model: the texts of a row differ from kind to kind.
    pub(crate) fn passes(&self, key: &ObjectKey, health: ObjectHealth) -> bool {
        let in_namespace =
            self.namespaces.is_empty() || key.namespace.is_empty() || self.namespaces.contains(&key.namespace);
        in_namespace && (!self.problems || health.is_problem())
    }
}

/// What the page shows of all objects besides the rows: each namespace with an object, in
/// the sequence of the alphabet.
#[derive(Debug, Default)]
pub(crate) struct Namespaces {
    pub(crate) names: Vec<Arc<str>>,
}

/// Collects the namespaces of the objects that the worker reads. The objects of a store
/// come by namespace, so one comparison finds a new one.
#[derive(Default)]
pub(crate) struct NamespaceSet<'a> {
    names: BTreeSet<Arc<str>>,
    last: Option<&'a Arc<str>>,
}

impl<'a> NamespaceSet<'a> {
    pub(crate) fn add(&mut self, key: &'a ObjectKey) {
        if self.last != Some(&key.namespace) {
            self.names.insert(Arc::clone(&key.namespace));
            self.last = Some(&key.namespace);
        }
    }

    /// The namespaces with an object. A kind with no namespace has none.
    pub(crate) fn done(self) -> Namespaces {
        Namespaces { names: self.names.into_iter().filter(|name| !name.is_empty()).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_namespace_and_problems_filters_let_an_object_pass_and_a_kind_with_no_namespace_has_none() {
        let key = ObjectKey::new("shop", "web");
        let mut filter = ObjectFilter::default();
        assert!(filter.passes(&key, ObjectHealth::Healthy));
        filter.problems = true;
        assert!(!filter.passes(&key, ObjectHealth::Progressing) && filter.passes(&key, ObjectHealth::Warning));
        filter.namespaces.insert("other".into());
        assert!(!filter.passes(&key, ObjectHealth::Failed));
        assert!(filter.passes(&ObjectKey::new("", "node-1"), ObjectHealth::Failed), "a node has no namespace");

        let keys = [ObjectKey::new("", "node-1"), ObjectKey::new("b", "x"), ObjectKey::new("a", "y")];
        let mut set = NamespaceSet::default();
        keys.iter().for_each(|key| set.add(key));
        assert_eq!(set.done().names, [Arc::from("a"), Arc::from("b")]);
        assert_eq!(ObjectFilter { text: "  Web ".into(), ..ObjectFilter::default() }.needle(), "web");
    }
}
