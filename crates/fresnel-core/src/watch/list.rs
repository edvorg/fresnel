//! The list of one kind, in pages.

use super::gate::Slot;
use super::lanes::{Lane, Readiness};
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
        let _slot = self.list_slot().await.map_err(Failure::Fault)?;
        self.start_request(StoreState::Listing { loaded: 0 });
        self.writer.begin_list();
        let mut token: Option<String> = None;
        loop {
            let (source, limit) = (&self.parts.source, self.parts.settings.page_size);
            let page = self.answer(source.list_page(limit, token.as_deref())).await?;
            let next = page.metadata.continue_.filter(|next| !next.is_empty());
            // A server that gives the token of the request back would get that request
            // again with no pause and with no end.
            if next.is_some() && next == token {
                return Err(same_token().into());
            }
            token = next;
            let version = page.metadata.resource_version.filter(|version| !version.is_empty());
            // With no version, the watch would start at "now" and lose each change
            // between the list and its own start, with no error.
            if token.is_none() && version.is_none() {
                return Err(no_version().into());
            }
            let loaded = self.writer.list_page(page.items.unwrap_or_default())?;
            self.writer.set_state(StoreState::Listing { loaded });
            if token.is_none() {
                self.attempts.listed();
                self.cursor = version;
                self.writer.end_list();
                return Ok(());
            }
        }
    }

    /// Waits for a list slot of the cluster and for a connection that carries the list.
    ///
    /// The wait for a slot can be long, and the connection can go in that time. A list
    /// with a slot and no connection would hold the slot to its time limit, and the store
    /// would count a failure that is the loss of the channel.
    async fn list_slot(&mut self) -> Result<Slot, CoreError> {
        loop {
            self.ready(Lane::Requests).await?;
            self.enter(StoreState::Waiting(Wait::ListSlot));
            let slot = self.parts.gate.take(self.parts.kind).await;
            if self.lanes.readiness(Lane::Requests, self.parts.clock.now()) == Readiness::Ready {
                return Ok(slot);
            }
        }
    }
}

fn no_version() -> CoreError {
    CoreError::invalid("the list of the server has no resource version")
}

fn same_token() -> CoreError {
    CoreError::invalid("the list of the server gave the token of a page two times")
}
