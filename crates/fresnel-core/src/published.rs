//! The one type that publishes a state to its readers (plan section 7.3).
//!
//! A task owns a [`Publisher`] and gives each new value of its state to it. A reader gets
//! the value as a copy of one pointer. It never gets a lock that can stop the task, and a
//! value that it holds does not change.
//!
//! A value can be large: the snapshot of a store holds each object of a kind. The last
//! holder of an old value frees it, and that can take many milliseconds. Thus a reader gets
//! a [`Held`] value, and the drop of it hands the value to the thread of `bin`. The UI
//! thread never frees an old value that it read.

mod bin;
mod notice;

use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::sync::Arc;

use tokio::sync::watch;

pub use bin::{Bin, Disposal};
pub use notice::{Changes, Notice};

/// The value that the readers see, and the mark of a publisher that closed.
struct Slot<T> {
    value: Arc<T>,
    closed: bool,
}

/// The write side of one state. A clone publishes to the same readers.
pub struct Publisher<T> {
    slot: watch::Sender<Slot<T>>,
    bin: Bin,
    notice: Notice,
}

impl<T> Clone for Publisher<T> {
    fn clone(&self) -> Self {
        Self { slot: self.slot.clone(), bin: self.bin.clone(), notice: self.notice.clone() }
    }
}

impl<T: Send + Sync + 'static> Publisher<T> {
    /// A state with its first value. `bin` frees the values that the readers held, and
    /// `notice` hears each publish: one notice can be for many states.
    pub fn new(first: T, bin: Bin, notice: Notice) -> Self {
        Self { slot: watch::Sender::new(Slot { value: Arc::new(first), closed: false }), bin, notice }
    }

    /// Gives a new value to the readers. A closed publisher keeps its last value.
    pub fn publish(&self, value: T) {
        let mut value = Arc::new(value);
        let published = self.slot.send_if_modified(|slot| {
            if slot.closed {
                return false;
            }
            std::mem::swap(&mut slot.value, &mut value);
            true
        });
        // The value before, or the new value of a closed publisher. Its free can take
        // long, so it comes here, after the lock of the slot: a reader never waits for it.
        drop(value);
        if published {
            self.notice.tell();
        }
    }

    /// Publishes the last value, which `last` makes from the current one. Each later
    /// publish does nothing. The owner of a task calls this after it stopped the task: a
    /// task that stops can run on to its next wait, and what it publishes then must not
    /// hide the final state.
    pub fn close(&self, last: impl FnOnce(&T) -> T) {
        let mut before = None;
        self.slot.send_modify(|slot| {
            let value = Arc::new(last(&slot.value));
            before = Some(std::mem::replace(&mut slot.value, value));
            slot.closed = true;
        });
        drop(before);
        self.notice.tell();
    }

    /// The current value.
    pub fn read(&self) -> Held<T> {
        Held::of(&self.slot.borrow().value, &self.bin)
    }

    /// A reader of the state.
    pub fn reader(&self) -> Reader<T> {
        Reader { slot: self.slot.subscribe(), bin: self.bin.clone() }
    }
}

/// The read side of one state. A clone is one more reader.
pub struct Reader<T> {
    slot: watch::Receiver<Slot<T>>,
    bin: Bin,
}

impl<T> Clone for Reader<T> {
    fn clone(&self) -> Self {
        Self { slot: self.slot.clone(), bin: self.bin.clone() }
    }
}

impl<T: Send + Sync + 'static> Reader<T> {
    /// The current value. The call copies one pointer and never waits for the publisher.
    pub fn read(&self) -> Held<T> {
        Held::of(&self.slot.borrow().value, &self.bin)
    }

    /// Waits for a value that this reader did not wait for before. It waits with no end
    /// after each publisher dropped.
    pub async fn changed(&mut self) {
        if self.slot.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// One value of a state, as a reader holds it. The value does not change.
pub struct Held<T: Send + Sync + 'static> {
    /// The pointer of this holder. Only the drop takes it out.
    value: ManuallyDrop<Arc<T>>,
    bin: Bin,
}

impl<T: Send + Sync + 'static> Held<T> {
    fn of(value: &Arc<T>, bin: &Bin) -> Self {
        Self { value: ManuallyDrop::new(Arc::clone(value)), bin: bin.clone() }
    }
}

impl<T: Send + Sync + 'static> Clone for Held<T> {
    fn clone(&self) -> Self {
        Self::of(&self.value, &self.bin)
    }
}

impl<T: Send + Sync + 'static> Deref for Held<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Send + Sync + 'static> Drop for Held<T> {
    /// Gives the pointer of this holder to the bin. The holder has no second pointer: with
    /// one more, the thread of the bin could drop its pointer first, and this thread would
    /// free the value. On Linux, a test found that sequence at once.
    fn drop(&mut self) {
        // SAFETY: the drop of the struct runs one time, and no code reads `value` after
        // this line: a field of the type `ManuallyDrop` has no drop of its own.
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        self.bin.put(value);
    }
}

#[cfg(test)]
mod tests;
