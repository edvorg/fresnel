//! The list of one kind, in pages.

use super::lanes::Lane;
use super::recover::Failure;
use super::source::Source;
use super::{Loop, StoreState, Wait};
use crate::error::CoreError;
use crate::kinds::Object;

impl<T: Object, S: Source<T>> Loop<'_, T, S> {
    /// Reads each page and then replaces the objects of the store in one step. The
    /// readers see the objects before the list until then.
    ///
    /// A list that fails starts again at its first page: the token of a page is for one
    /// version of the data, and a list of two versions can lose an object.
    pub(super) async fn list(&mut self) -> Result<(), Failure> {
        self.ready(Lane::Requests).await.map_err(Failure::Fault)?;
        self.enter(StoreState::Waiting(Wait::ListSlot));
        let _slot = self.parts.gate.take(self.parts.kind).await;
        self.start_request(StoreState::Listing { loaded: 0 });
        self.writer.begin_list();
        let mut token: Option<String> = None;
        loop {
            let (source, limit) = (&self.parts.source, self.parts.settings.page_size);
            let page = self.answer(source.list_page(limit, token.as_deref())).await?;
            token = page.metadata.continue_.filter(|token| !token.is_empty());
            let version = page.metadata.resource_version.filter(|version| !version.is_empty());
            // With no version, the watch would start at "now" and lose each change
            // between the list and its own start, with no error.
            if token.is_none() && version.is_none() {
                return Err(no_version().into());
            }
            let loaded = self.writer.list_page(page.items.unwrap_or_default())?;
            // A page that the store can use is the answer that ends a sequence of failures.
            self.attempts.answered();
            self.writer.set_state(StoreState::Listing { loaded });
            if token.is_none() {
                self.cursor = version;
                self.writer.end_list();
                return Ok(());
            }
        }
    }
}

fn no_version() -> CoreError {
    CoreError::invalid("the list of the server has no resource version")
}
