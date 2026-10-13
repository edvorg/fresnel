//! The rows of one request, and the request itself. The worker thread of a table makes
//! the first from the second.

use super::model::{Computed, Model, Sort};
use super::worker::Cancel;

/// One request to the worker thread: what the rows come from.
pub(super) struct Request<M: Model> {
    pub(super) sort: Sort,
    pub(super) filter: M::Filter,
    pub(super) data: M::Data,
}

impl<M: Model> Clone for Request<M> {
    fn clone(&self) -> Self {
        Self { sort: self.sort, filter: self.filter.clone(), data: self.data.clone() }
    }
}

/// The rows of one request, in the sequence of its sort.
pub(crate) struct Rows<M: Model> {
    pub(crate) rows: Vec<M::Row>,
    /// The count of objects before the filter.
    pub(crate) total: usize,
    pub(crate) summary: M::Summary,
}

/// The rows of a request in the sequence of its sort. It runs on the worker thread.
pub(super) fn sorted_rows<M: Model>(model: &M, request: &Request<M>, cancel: &Cancel<'_>) -> Option<Rows<M>> {
    let Computed { mut rows, total, summary } = model.rows(&request.data, &request.filter, cancel)?;
    if cancel.is_set() {
        return None;
    }
    let Sort { column, descending } = request.sort;
    rows.sort_unstable_by(|a, b| {
        let order = model.compare(a, b, column);
        // The last key has one direction for each sort, so rows with equal values keep
        // their sequence when the user turns the sort.
        (if descending { order.reverse() } else { order }).then_with(|| model.tie(a, b))
    });
    Some(Rows { rows, total, summary })
}
