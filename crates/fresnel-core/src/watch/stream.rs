//! One watch: the events of the server from one resource version to the end of the stream.

use std::time::Duration;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use tokio::time::{Instant, sleep_until};

use super::lanes::Lane;
use super::recover::Failure;
use super::source::{Events, Source};
use super::{Loop, StoreState, Wait};
use crate::cluster::WatchEvent;
use crate::error::CoreError;
use crate::kinds::Object;
use crate::net::ErrorClass;

/// How a watch ended with no failure of a request.
enum End {
    /// The server ended the stream. It does so at the timeout of each watch.
    ByServer,
    /// No channel for watches is live: the connection of the stream is gone.
    ChannelLost,
}

impl<T: Object, S: Source<T>> Loop<'_, T, S> {
    /// Runs one watch from the resource version `from` to its end. The stream is closed
    /// at the return, so no stream of the connection stays in use while the loop waits.
    pub(super) async fn watch(&mut self, from: &str) -> Result<(), Failure> {
        let renewal = std::mem::take(&mut self.renewal);
        self.ready(Lane::Watches).await.map_err(Failure::Fault)?;
        self.check_cluster(from).await?;
        // A store stays live while the renewal of its watch starts. Each other start of a
        // watch shows as a state, and so does a renewal that had to wait before its request.
        if !(renewal && self.writer.state().is_live()) {
            self.start_request(StoreState::Waiting(Wait::Stream));
        }
        let range = self.parts.settings.server_timeout.clone();
        let timeout = fastrand::u32(range);
        let source = &self.parts.source;
        let mut events = self.answer(Lane::Watches, source.watch(from, timeout)).await?;
        // The good status of the server is the answer. From now on each change arrives.
        self.writer.set_state(StoreState::Live);

        let opened = Instant::now();
        let stall = Duration::from_secs(timeout.into()) + self.parts.settings.stall_margin;
        let mut proved = false;
        let end = self.follow(&mut events, stall, &mut proved).await;
        drop(events);
        // A watch that gave data or stayed open did work, however it ended.
        let worked = proved || opened.elapsed() >= self.parts.settings.backoff.reset_after;
        if worked {
            self.attempts.watch_worked();
        }
        match end {
            Ok(End::ChannelLost) => Err(Failure::ChannelLost),
            // A server that ends each watch at once must not get a new one with no pause.
            Ok(End::ByServer) if !worked => Err(Failure::Failed(ended_early(opened.elapsed()))),
            // The usual end: the server ends each watch at its timeout. The next watch is
            // the renewal of this one. The state does not change, so a change that waits
            // for its batch shows here and not after the next request.
            Ok(End::ByServer) => {
                self.renewal = true;
                self.writer.flush();
                Ok(())
            }
            // The usual cause is the loss of the connection, and the channel counts that
            // loss one time (plan section 8.13). The next watch starts with no pause.
            Err(Failure::Failed(error)) if worked => Err(Failure::Interrupted(error)),
            Err(failure) => Err(failure),
        }
    }

    /// Tells before each watch whether the cluster behind the connections is still the cluster
    /// of the objects of the store. The store shows "checks the cluster" until then: a soak saw
    /// a store "live" with the objects of an old cluster for 20 s after a recreate.
    ///
    /// Each new connection can reach an other cluster at the same address, also one that the
    /// store did not see go, in a pause. So no watch starts on a connection after those of the
    /// last list or check before the task of the identity read the UID of `kube-system` over
    /// it: an other UID gives a new list (`identity`). After a lost connection, the version of
    /// the server comes next.
    async fn check_cluster(&mut self, from: &str) -> Result<(), Failure> {
        if self.lost {
            self.ready(Lane::Requests).await.map_err(Failure::Fault)?;
        }
        if self.lanes.connects() > self.connects {
            self.connects = self.identify().await.map_err(Failure::Fault)?;
        }
        if self.lanes.cluster() != self.cluster {
            return Err(Failure::Gone(other_cluster()));
        }
        if self.lost {
            self.check_version(from).await?;
            self.lost = false;
        }
        Ok(())
    }

    /// Waits until the task of the identity read the cluster behind each connection of now,
    /// and gives their count. The read goes out on the lane of requests, so the store shows a
    /// wait for that lane first.
    async fn identify(&mut self) -> Result<u64, CoreError> {
        loop {
            let connects = self.lanes.connects();
            if self.lanes.is_identified(connects) {
                return Ok(connects);
            }
            self.ready(Lane::Requests).await?;
            let connects = self.lanes.connects();
            if !self.lanes.is_identified(connects) {
                self.enter(StoreState::Waiting(Wait::Cluster));
                self.lanes.identified(connects).await?;
            }
        }
    }

    /// Tells after a lost connection whether the cluster went back to an older version: a
    /// restore of a backup. Such a server takes a watch from a version that it does not have
    /// yet and sends nothing until its own version passes it. A list of one object gives the
    /// version of the server now; a version older than the cursor gives a new list, as HTTP
    /// 410 does. A new cluster that a user cannot read `kube-system` of can pass the cursor
    /// first: its first object then has the name of an object of the store and an other UID,
    /// which gives a new list too.
    async fn check_version(&mut self, from: &str) -> Result<(), Failure> {
        self.start_request(StoreState::Waiting(Wait::Cluster));
        let source = &self.parts.source;
        let page = self.answer(Lane::Requests, source.list_page(1, None)).await?;
        let number = |version: &str| version.parse::<u128>().ok();
        let now = page.metadata.resource_version.as_deref().and_then(number);
        if let (Some(now), Some(from)) = (now, number(from))
            && now < from
        {
            return Err(Failure::Gone(went_back(now, from)));
        }
        let first = page.items.iter().flatten().next();
        match first.filter(|object| self.writer.holds_other(object.meta())) {
            Some(object) => Err(Failure::Gone(other_uid(object.meta()))),
            None => Ok(()),
        }
    }

    /// Applies the events of one stream. `proved` gets true at the first event or
    /// bookmark: the watch then showed that it works.
    async fn follow(&mut self, events: &mut S::Events, stall: Duration, proved: &mut bool) -> Result<End, Failure> {
        let mut stalled_at = Instant::now() + stall;
        let mut identity = self.lanes.clone();
        loop {
            let due = self.writer.due();
            tokio::select! {
                event = events.next() => {
                    let Some(event) = event else { return Ok(End::ByServer) };
                    self.apply(event?)?;
                    *proved = true;
                    stalled_at = Instant::now() + stall;
                }
                () = wait_until(due) => self.writer.publish_if_due(),
                () = self.lanes.lost(Lane::Watches) => return Ok(End::ChannelLost),
                () = identity.other_cluster(self.cluster) => return Err(Failure::Gone(other_cluster())),
                () = sleep_until(stalled_at) => return Err(Failure::Failed(stalled(stall))),
            }
        }
    }

    /// Applies one event. The cursor follows each event and each bookmark that the loop
    /// can read, so the next watch starts after it. That holds also for an object that
    /// the store refuses, one with no name: the same event would fail again with no end.
    fn apply(&mut self, event: WatchEvent<T>) -> Result<(), Failure> {
        match event {
            WatchEvent::Added(object) | WatchEvent::Modified(object) => {
                self.follow_version(object.meta().resource_version.as_deref());
                self.writer.apply(object)?;
            }
            WatchEvent::Deleted(object) => {
                self.follow_version(object.meta().resource_version.as_deref());
                self.writer.delete(&object)?;
            }
            WatchEvent::Bookmark(bookmark) => {
                self.follow_version(Some(&bookmark.metadata.resource_version));
                self.writer.contact();
            }
            WatchEvent::Error(status) => return Err(Failure::of_event(&status)),
        }
        self.writer.publish_if_due();
        Ok(())
    }

    fn follow_version(&mut self, version: Option<&str>) {
        if let Some(version) = version.filter(|version| !version.is_empty()) {
            self.cursor = Some(version.to_owned());
        }
    }
}

/// Waits until the time. With no time, the wait has no end.
async fn wait_until(time: Option<Instant>) {
    match time {
        Some(time) => sleep_until(time).await,
        None => std::future::pending().await,
    }
}

fn stalled(limit: Duration) -> CoreError {
    let text = format!("the watch gave no event and no bookmark in {} s", limit.as_secs());
    CoreError::new(ErrorClass::Timeout, text)
}

fn went_back(now: u128, cursor: u128) -> CoreError {
    let text = format!("the cluster went back to the version {now}, before the version {cursor} of the store");
    CoreError::new(ErrorClass::Server, text)
}

fn other_uid(meta: &ObjectMeta) -> CoreError {
    let name = meta.name.as_deref().unwrap_or_default();
    let text = format!("the object {name} of the cluster has an other UID than the object of the store");
    CoreError::new(ErrorClass::Server, text)
}

fn other_cluster() -> CoreError {
    CoreError::new(ErrorClass::Server, "the cluster is an other one: its Namespace kube-system has an other UID")
}

/// The server ended the stream: the connection works, so this is no lost connection.
fn ended_early(after: Duration) -> CoreError {
    let text = format!("the server ended the watch after {} ms with no event", after.as_millis());
    CoreError::new(ErrorClass::Server, text)
}
