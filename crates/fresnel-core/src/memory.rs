//! The memory account of the core: the size of each bounded part at one moment (design
//! rule 12). A soak records it each minute, so a part that grows tells its name.
//!
//! A part gives its items and its bytes. A buffer that counts its bytes for its limit, as
//! the ring of a log view and the history of the metrics, gives that count; a record of a
//! known shape gives its count times its size. The objects of a store have no known size:
//! a [`Scale`] of the program measures a copy of them.

/// One part of the core at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// What the part is, with its cluster: `default: store Pod`.
    pub name: String,
    /// Its items: objects, lines, series, entries or records.
    pub items: usize,
    /// Its bytes on the heap: the count of the part itself, or the measure of the scale.
    pub bytes: usize,
}

/// Measures the bytes on the heap of a value that the core cannot count. The soak has one on
/// its counting allocator. [`NoScale`] gives 0 bytes for such a part, and its items stay.
pub trait Scale {
    /// The bytes that `copy` allocates on the thread of the call.
    fn bytes(&self, copy: &mut dyn FnMut()) -> usize;
}

/// The scale of a program with no counting allocator.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoScale;

impl Scale for NoScale {
    fn bytes(&self, _copy: &mut dyn FnMut()) -> usize {
        0
    }
}

/// The parts of the walk so far. Each part of a cluster has the name of its cluster first.
pub struct Account<'a> {
    parts: Vec<Part>,
    scale: &'a dyn Scale,
    /// The start of the name of each part that follows.
    scope: String,
}

impl<'a> Account<'a> {
    /// An account with no part, which measures with `scale`.
    pub fn new(scale: &'a dyn Scale) -> Self {
        Self { parts: Vec::new(), scale, scope: String::new() }
    }

    /// Adds one part.
    pub(crate) fn add(&mut self, name: &str, items: usize, bytes: usize) {
        self.parts.push(Part { name: format!("{}{name}", self.scope), items, bytes });
    }

    /// The bytes that `copy` allocates, by the scale of the program.
    pub(crate) fn measure(&self, copy: &mut dyn FnMut()) -> usize {
        self.scale.bytes(copy)
    }

    /// Runs `walk` with the name of a cluster before each part that it adds.
    pub(crate) fn scoped(&mut self, scope: &str, walk: impl FnOnce(&mut Self)) {
        let outer = std::mem::replace(&mut self.scope, format!("{scope}: "));
        walk(self);
        self.scope = outer;
    }

    /// The parts in the sequence of the walk.
    pub fn parts(self) -> Vec<Part> {
        self.parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;

    impl Scale for Fixed {
        fn bytes(&self, copy: &mut dyn FnMut()) -> usize {
            copy();
            7
        }
    }

    #[test]
    fn a_part_of_a_cluster_has_its_name_first_and_the_scale_measures_a_copy() {
        let mut account = Account::new(&Fixed);
        account.add("journal", 3, 300);
        let mut copied = false;
        account.scoped("ams", |account| {
            let bytes = account.measure(&mut || copied = true);
            account.add("store Pod", 2, bytes);
        });
        account.add("tasks", 1, 10);
        let names: Vec<(String, usize, usize)> =
            account.parts().into_iter().map(|part| (part.name, part.items, part.bytes)).collect();
        assert!(copied);
        let wanted = [("journal", 3, 300), ("ams: store Pod", 2, 7), ("tasks", 1, 10)];
        assert_eq!(names, wanted.map(|(name, items, bytes)| (name.to_owned(), items, bytes)));
        assert_eq!(NoScale.bytes(&mut || {}), 0);
    }
}
