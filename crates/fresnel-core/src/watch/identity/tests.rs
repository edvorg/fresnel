use std::collections::VecDeque;
use std::sync::Mutex;

use tokio::time::timeout;

use super::*;
use crate::net::ErrorClass;
use crate::watch::lanes::Path;

const LIVE: LaneStates = LaneStates {
    requests: Path::Live,
    watches: Path::Live,
    credential_failed: false,
    throttled_until: None,
    connects: 0,
    identity: Identity { number: 0, checked: 0 },
};

/// The answers of the server to the reads of the UID, in their sequence. A read with no
/// answer waits with no end.
struct Script {
    answers: Mutex<VecDeque<Result<Option<Arc<str>>, CoreError>>>,
    reads: watch::Sender<usize>,
}

impl Script {
    fn new(answers: Vec<Result<&str, CoreError>>) -> Self {
        let answers = answers.into_iter().map(|answer| answer.map(|uid| Some(Arc::from(uid)))).collect();
        Self { answers: Mutex::new(answers), reads: watch::Sender::new(0) }
    }

    fn read(&self) -> impl Future<Output = Result<Option<Arc<str>>, CoreError>> + use<> {
        let answer = self.answers.lock().expect("a lock").pop_front();
        self.reads.send_modify(|reads| *reads += 1);
        async move {
            match answer {
                Some(answer) => answer,
                None => std::future::pending().await,
            }
        }
    }

    async fn until_reads(&self, count: usize) {
        let mut reads = self.reads.subscribe();
        let waited = timeout(Duration::from_secs(3600), reads.wait_for(|reads| *reads == count)).await;
        assert!(waited.is_ok_and(|read| read.is_ok()), "{count} reads");
    }
}

/// The connections go for a second and come back as new ones: the count grows to `connects`.
async fn lost_and_back(states: &watch::Sender<LaneStates>, connects: u64) {
    states.send_modify(|states| (states.requests, states.watches) = (Path::NoChannel, Path::NoChannel));
    sleep(Duration::from_secs(1)).await;
    states
        .send_modify(|states| (states.requests, states.watches, states.connects) = (Path::Live, Path::Live, connects));
}

/// Waits until the task published that it read the cluster after `connects` connections.
async fn until_checked(states: &watch::Sender<LaneStates>, connects: u64) -> Identity {
    let mut changed = states.subscribe();
    let checked =
        timeout(Duration::from_secs(60), changed.wait_for(|states| states.identity.checked == connects)).await;
    checked.expect("in time").expect("the states").identity
}

/// The UID of `kube-system` is read after each new connection, and only an other UID gives
/// the cluster a new number. Each read tells the count of connections that it covers, for the
/// stores that wait for it. A read that fails comes again after a second.
#[tokio::test(start_paused = true)]
async fn an_other_uid_of_kube_system_after_a_new_connection_gives_the_cluster_a_new_number() {
    let timed_out = CoreError::new(ErrorClass::Timeout, "Get request: no answer in 10s");
    let script = Script::new(vec![Ok("a"), Ok("a"), Err(timed_out), Ok("b")]);
    let states = watch::Sender::new(LaneStates { requests: Path::NoChannel, ..LIVE });
    let steps = async {
        sleep(Duration::from_secs(5)).await;
        assert_eq!(*script.reads.borrow(), 0, "no read before the channel of requests is live");
        states.send_replace(LaneStates { connects: 3, ..LIVE });
        assert_eq!(until_checked(&states, 3).await, Identity { number: 0, checked: 3 }, "the first read");
        sleep(Duration::from_secs(60)).await;
        assert_eq!(*script.reads.borrow(), 1, "no read with no new connection");

        lost_and_back(&states, 4).await;
        assert_eq!(until_checked(&states, 4).await, Identity { number: 0, checked: 4 }, "the same cluster");
        lost_and_back(&states, 5).await;
        let failed_at = tokio::time::Instant::now();
        script.until_reads(3).await;
        assert_eq!(states.borrow().identity.checked, 4, "a read that failed tells nothing");
        assert_eq!(until_checked(&states, 5).await, Identity { number: 1, checked: 5 }, "an other cluster");
        assert_eq!(failed_at.elapsed(), RETRY, "the next read after a pause");
        assert_eq!(states.borrow().requests, Path::Live, "the identity changes nothing else");
    };
    tokio::select! {
        ended = follow(|| script.read(), &states) => panic!("the task ended: {ended:?}"),
        () = steps => {}
    }
}

/// A new connection ends the pause after a read that failed: the stores wait for the read.
#[tokio::test(start_paused = true)]
async fn a_new_connection_ends_the_pause_after_a_read_that_failed() {
    let broke = CoreError::new(ErrorClass::Network, "request channel: the connection ended before the answer");
    let script = Script::new(vec![Ok("a"), Err(broke), Ok("a")]);
    let states = watch::Sender::new(LaneStates { connects: 3, ..LIVE });
    let steps = async {
        until_checked(&states, 3).await;
        states.send_modify(|states| states.connects = 4);
        script.until_reads(2).await;
        sleep(Duration::from_millis(100)).await;
        states.send_modify(|states| states.connects = 5);
        assert_eq!(until_checked(&states, 5).await, Identity { number: 0, checked: 5 });
        assert_eq!(*script.reads.borrow(), 3, "one read for the connection after the failure");
    };
    tokio::select! {
        ended = follow(|| script.read(), &states) => panic!("the task ended: {ended:?}"),
        () = steps => {}
    }
}

/// A user who cannot read `kube-system` gets no check: each read lets the stores go on, and
/// the cluster keeps its number.
#[tokio::test(start_paused = true)]
async fn a_refused_read_lets_the_stores_go_on_with_no_check() {
    let refused = || CoreError::answered(403, "forbidden");
    let script = Script::new(vec![Err(refused()), Err(refused())]);
    let states = watch::Sender::new(LaneStates { connects: 3, ..LIVE });
    let steps = async {
        assert_eq!(until_checked(&states, 3).await, Identity { number: 0, checked: 3 });
        lost_and_back(&states, 4).await;
        assert_eq!(until_checked(&states, 4).await, Identity { number: 0, checked: 4 });
        sleep(Duration::from_secs(60)).await;
        assert_eq!(*script.reads.borrow(), 2, "one read for each new connection");
    };
    tokio::select! {
        ended = follow(|| script.read(), &states) => panic!("the task ended: {ended:?}"),
        () = steps => {}
    }
}

/// Each cluster has `kube-system` for its life. A new cluster in its start can have none
/// yet: it is an other cluster, and the next read takes its UID for the next check.
#[tokio::test(start_paused = true)]
async fn a_cluster_with_no_kube_system_after_a_known_uid_is_an_other_one() {
    let script = Script::new(vec![Ok("a"), Err(CoreError::answered(404, "not found")), Ok("b"), Ok("c")]);
    let states = watch::Sender::new(LaneStates { connects: 3, ..LIVE });
    let steps = async {
        until_checked(&states, 3).await;
        lost_and_back(&states, 4).await;
        assert_eq!(until_checked(&states, 4).await, Identity { number: 1, checked: 4 }, "an other cluster");
        script.until_reads(3).await;
        sleep(Duration::from_secs(1)).await;
        assert_eq!(states.borrow().identity.number, 1, "the UID of the new cluster is no change");
        lost_and_back(&states, 5).await;
        assert_eq!(until_checked(&states, 5).await, Identity { number: 2, checked: 5 }, "a third cluster");
    };
    tokio::select! {
        ended = follow(|| script.read(), &states) => panic!("the task ended: {ended:?}"),
        () = steps => {}
    }
}
