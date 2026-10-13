//! The thread that frees the old values of the published states.
//!
//! A snapshot of 10,000 pods that changed needs 22 ms for its free in the store prototype,
//! more than one frame. A reader puts each value that it held into the bin, and one worker
//! thread empties the bin.

use std::any::Any;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use fresnel_net::task::{Backoff, Restart, Stop, TaskHandle, TaskName, Tasks};

/// A value that some reader held, with no type.
pub(super) type Old = Arc<dyn Any + Send + Sync>;

/// The most values that wait for the thread. A reader that finds the bin full frees its
/// value itself, and the counter of the bin shows it.
const LIMIT: usize = 4096;
/// The thread rests this long after it emptied the bin. Thus the values of many frames
/// cost few wakes, and an old value lives this much longer at most.
const REST: Duration = Duration::from_millis(20);

struct Queue {
    values: Vec<Old>,
    /// False after the owner of the thread dropped. Then each holder frees its own value.
    open: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    filled: Condvar,
    freed_in_place: AtomicU64,
}

impl Shared {
    /// Each use holds the lock for one push or one swap. No value drops under the lock.
    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The body of the thread: takes all values at once and frees them outside the lock.
    fn empty_until_closed(&self) {
        loop {
            let values = {
                let mut queue = self.queue();
                while queue.values.is_empty() && queue.open {
                    queue = self.filled.wait(queue).unwrap_or_else(PoisonError::into_inner);
                }
                if queue.values.is_empty() {
                    return;
                }
                std::mem::take(&mut queue.values)
            };
            drop(values);
            std::thread::sleep(REST);
        }
    }
}

/// Takes the values that readers held. A clone puts into the same bin.
#[derive(Clone)]
pub struct Bin {
    shared: Arc<Shared>,
}

impl Bin {
    /// Hands one pointer of a value to the thread. The call holds a lock for one push.
    pub(super) fn put(&self, value: Old) {
        let mut queue = self.shared.queue();
        if !queue.open || queue.values.len() >= LIMIT {
            let full = queue.open;
            drop(queue);
            self.shared.freed_in_place.fetch_add(u64::from(full), Ordering::Relaxed);
            // `value` drops here, on the thread of the caller.
            return;
        }
        let wake = queue.values.is_empty();
        queue.values.push(value);
        drop(queue);
        if wake {
            self.shared.filled.notify_one();
        }
    }
}

/// Closes the bin at its drop, which ends the thread.
struct Door(Bin);

impl Drop for Door {
    fn drop(&mut self) {
        self.0.shared.queue().open = false;
        self.0.shared.filled.notify_all();
    }
}

/// The owner of the thread that frees old values. The thread ends when this value drops.
///
/// The task handle is the first field: it drops before the door, so the end of the thread
/// is the stop of its task and not a fault.
pub struct Disposal {
    _task: TaskHandle,
    door: Door,
}

impl Disposal {
    /// Starts the thread.
    pub fn start(tasks: &Tasks) -> Self {
        let queue = Queue { values: Vec::new(), open: true };
        let shared =
            Arc::new(Shared { queue: Mutex::new(queue), filled: Condvar::new(), freed_in_place: AtomicU64::new(0) });
        let body = {
            let shared = Arc::clone(&shared);
            move |_stop: &Stop| {
                shared.empty_until_closed();
                Ok::<(), Infallible>(())
            }
        };
        let task = tasks.start_thread(TaskName::new("old snapshots"), Restart::Always(Backoff::default()), body);
        Self { _task: task, door: Door(Bin { shared }) }
    }

    /// The bin of the thread, for a [`Publisher`](super::Publisher).
    pub fn bin(&self) -> Bin {
        self.door.0.clone()
    }

    /// The count of values that a reader freed itself, because the bin was full.
    pub fn freed_in_place(&self) -> u64 {
        self.door.0.shared.freed_in_place.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
pub(super) const TEST_LIMIT: usize = LIMIT;
