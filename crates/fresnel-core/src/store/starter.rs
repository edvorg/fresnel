//! The start of one store with its task, for a store that is always on and for one on
//! demand.

use std::any::Any;
use std::sync::Arc;

use fresnel_net::task::{Restart, TaskHandle, TaskName, Tasks};
use fresnel_net::{Signal, WallClock};

use super::cluster::StoreParts;
use super::snapshot::{Snapshot, StoreStatus};
use super::state::{StoreState, Wait};
use crate::cluster::{ClusterId, Gateway};
use crate::kinds::{Kind, Object};
use crate::published::{Notice, Publisher, Reader};
use crate::watch::{self, GatewaySource, Lanes, ListGate, Parts, WatchSettings};

/// One store with no type of its objects.
trait AnyStore: Send + Sync {
    fn status(&self) -> StoreStatus;

    /// Publishes the final state. What the task publishes after it does not show.
    fn stop(&self);

    fn as_any(&self) -> &dyn Any;
}

struct Store<T> {
    kind: Kind,
    publisher: Publisher<Snapshot<T>>,
    clock: WallClock,
}

impl<T: Object> AnyStore for Store<T> {
    fn status(&self) -> StoreStatus {
        self.publisher.read().status(self.kind)
    }

    fn stop(&self) {
        self.publisher.close(|last| last.in_state(StoreState::Stopped, self.clock.now()));
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A store with its task. The task stops when this value drops, and the readers of the
/// store then see the state "stopped" with the last objects.
pub(super) struct Running {
    pub(super) kind: Kind,
    store: Box<dyn AnyStore>,
    _task: TaskHandle,
}

impl Running {
    pub(super) fn status(&self) -> StoreStatus {
        self.store.status()
    }

    /// A reader of the objects. `None` when the store holds objects of an other type.
    pub(super) fn reader<T: Object>(&self) -> Option<Reader<Snapshot<T>>> {
        let store = self.store.as_any().downcast_ref::<Store<T>>()?;
        Some(store.publisher.reader())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.store.stop();
    }
}

/// What the server sends of each object of a store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Form {
    /// The full object.
    Full,
    /// The metadata of the object only: the store of a tier B kind that is always on.
    Metadata,
}

/// What a store needs that the other stores of its cluster share.
pub(super) struct Starter {
    pub(super) tasks: Tasks,
    pub(super) gateway: Gateway,
    pub(super) signal: Signal,
    pub(super) lanes: Lanes,
    pub(super) gate: Arc<ListGate>,
    pub(super) watch: Arc<WatchSettings>,
    pub(super) parts: StoreParts,
    pub(super) notice: Notice,
}

impl Starter {
    pub(super) fn cluster(&self) -> &ClusterId {
        self.gateway.cluster()
    }

    /// Starts the store of one kind with one task. `fields` is the field selector of its
    /// requests, and `max_items` the most objects that it keeps.
    pub(super) fn start<T: Object>(
        &self,
        kind: Kind,
        form: Form,
        fields: Option<&'static str>,
        max_items: Option<usize>,
    ) -> Running {
        let clock = self.parts.clock.clone();
        let first = Snapshot::<T>::empty(StoreState::Waiting(Wait::Channel), clock.now());
        let publisher = Publisher::new(first, self.parts.bin.clone(), self.notice.clone());
        let source = GatewaySource::new(self.gateway.clone(), kind.info(), form == Form::Metadata, fields);
        let parts = Arc::new(Parts {
            kind,
            source,
            publisher: publisher.clone(),
            lanes: self.lanes.clone(),
            signal: self.signal.clone(),
            gate: Arc::clone(&self.gate),
            settings: Arc::clone(&self.watch),
            clock: clock.clone(),
            max_items,
        });
        let name = TaskName::scoped("store", format!("{}/{}", self.cluster(), kind.info().name));
        let restart = Restart::Always(self.parts.settings.restart);
        let task = self.tasks.start(name, restart, move || watch::run(Arc::clone(&parts)));
        Running { kind, store: Box::new(Store { kind, publisher, clock }), _task: task }
    }
}
