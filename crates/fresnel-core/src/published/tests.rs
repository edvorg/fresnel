use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use fresnel_net::task::{TaskReport, Tasks};
use futures_util::FutureExt as _;
use tokio::runtime::Handle;

use super::*;

const LIMIT: Duration = Duration::from_secs(30);

/// The tasks of one test, with the reports of the runs that ended.
fn tasks() -> (Tasks, Arc<Mutex<Vec<String>>>) {
    let ended = Arc::new(Mutex::new(Vec::new()));
    let reports = Arc::clone(&ended);
    let tasks = Tasks::new(Handle::current(), move |report: TaskReport| {
        reports.lock().unwrap_or_else(PoisonError::into_inner).push(report.to_string());
    });
    (tasks, ended)
}

/// A value that tells which thread frees it. With a gate, the free waits for the test.
struct Tracked {
    freed_on: Sender<Option<String>>,
    gate: Option<Mutex<Receiver<()>>>,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.freed_on.send(std::thread::current().name().map(str::to_owned)).expect("the test holds the receiver");
        if let Some(gate) = &self.gate {
            gate.lock().expect("a lock").recv_timeout(LIMIT).expect("the test opens the gate");
        }
    }
}

#[tokio::test]
async fn a_reader_gets_each_new_value_and_a_value_that_it_holds_does_not_change() {
    let (tasks, ended) = tasks();
    let disposal = Disposal::start(&tasks);
    let notice = Notice::new();
    let mut of_group = notice.changes();
    let publisher = Publisher::new(1, disposal.bin(), notice.clone());
    let mut reader = publisher.reader();
    let first = reader.read();
    assert!(reader.changed().now_or_never().is_none(), "the first value is not a change");
    assert!(of_group.next().now_or_never().is_none());

    publisher.publish(2);
    reader.changed().await;
    assert_eq!((*first, *reader.read(), *publisher.read(), *first.clone()), (1, 2, 2, 1));
    assert!(reader.changed().now_or_never().is_none(), "one publish is one change");

    // One notice is for many states: a reader of all of them waits in one place.
    let other = Publisher::new("a", disposal.bin(), notice);
    assert!(of_group.next().now_or_never().is_some(), "the publish of the first state");
    assert!(of_group.next().now_or_never().is_none());
    other.publish("b");
    assert!(of_group.next().now_or_never().is_some());

    // A task that was stopped can publish one more time. The final value stays.
    publisher.close(|last| last + 40);
    publisher.publish(3);
    assert_eq!(*reader.clone().read(), 42);
    assert!(of_group.next().now_or_never().is_some(), "the final value is a change");
    assert!(of_group.next().now_or_never().is_none(), "a publish that does not show is not a change");
    drop((first, publisher, disposal));
    assert_eq!(*ended.lock().expect("a lock"), Vec::<String>::new(), "the end of the thread is not a fault");
}

#[tokio::test]
async fn the_thread_of_the_bin_frees_a_value_that_a_reader_held() {
    let (tasks, ended) = tasks();
    let disposal = Disposal::start(&tasks);
    let (freed_on, freed) = channel();
    let tracked = || Tracked { freed_on: freed_on.clone(), gate: None };
    let publisher = Publisher::new(tracked(), disposal.bin(), Notice::new());

    // No reader holds the first value: the publisher frees it, here on the thread of the test.
    publisher.publish(tracked());
    let here = std::thread::current().name().map(str::to_owned);
    assert_eq!(freed.recv_timeout(LIMIT).expect("the first value is free"), here);

    // A reader holds the second value over the next publish, as a frame of the UI does.
    let held = publisher.reader().read();
    publisher.publish(tracked());
    assert!(freed.try_recv().is_err(), "a value that a reader holds stays");
    drop(held);
    let thread = freed.recv_timeout(LIMIT).expect("the second value is free");
    assert_eq!(thread.as_deref(), Some("old snapshots"), "the reader did not free the value itself");
    assert_eq!(disposal.freed_in_place(), 0);
    assert_eq!(*ended.lock().expect("a lock"), Vec::<String>::new());
}

/// Design rule 12: the bin has a limit, and a value that does not fit shows in a counter.
#[tokio::test]
async fn a_reader_frees_its_value_itself_when_the_bin_is_full_or_closed() {
    let (tasks, _ended) = tasks();
    let disposal = Disposal::start(&tasks);
    let (freed_on, freed) = channel();
    let (open_gate, gate) = channel();
    let slow = Publisher::new(
        Tracked { freed_on: freed_on.clone(), gate: Some(Mutex::new(gate)) },
        disposal.bin(),
        Notice::new(),
    );
    let held = slow.read();
    slow.publish(Tracked { freed_on: freed_on.clone(), gate: None });
    drop(held);
    freed.recv_timeout(LIMIT).expect("the thread started the free of the slow value");

    // The thread is in that free, so the bin fills.
    let numbers = Publisher::new(7_u32, disposal.bin(), Notice::new());
    let many: Vec<Held<u32>> = (0..bin::TEST_LIMIT + 3).map(|_| numbers.read()).collect();
    drop(many);
    assert_eq!(disposal.freed_in_place(), 3, "the values above the limit");
    open_gate.send(()).expect("the thread waits at the gate");

    // After the owner of the thread dropped, a holder frees its value with no count.
    let bin = disposal.bin();
    drop(disposal);
    let late = Publisher::new(Tracked { freed_on: freed_on.clone(), gate: None }, bin, Notice::new());
    let held = late.read();
    late.publish(Tracked { freed_on, gate: None });
    drop(held);
    let here = std::thread::current().name().map(str::to_owned);
    assert_eq!(freed.recv_timeout(LIMIT).expect("the late value is free"), here);
    drop(late);
    assert_eq!(freed.recv_timeout(LIMIT).expect("the last value is free"), here);
}
