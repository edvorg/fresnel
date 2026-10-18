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
    cluster: 0,
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

/// The connections go for a second and come back.
async fn lost_and_back(states: &watch::Sender<LaneStates>) {
    states.send_modify(|states| (states.requests, states.watches) = (Path::NoChannel, Path::NoChannel));
    sleep(Duration::from_secs(1)).await;
    states.send_modify(|states| (states.requests, states.watches) = (Path::Live, Path::Live));
}

/// The UID of `kube-system` is read at the connect and after each lost connection, and only
/// an other UID gives the cluster a new number. A read that fails tries again after a pause.
#[tokio::test(start_paused = true)]
async fn an_other_uid_of_kube_system_after_a_lost_connection_gives_the_cluster_a_new_number() {
    let timed_out = CoreError::new(ErrorClass::Timeout, "Get request: no answer in 10s");
    let script = Script::new(vec![Ok("a"), Ok("a"), Err(timed_out), Ok("b")]);
    let states = watch::Sender::new(LaneStates { requests: Path::NoChannel, ..LIVE });
    let steps = async {
        sleep(Duration::from_secs(5)).await;
        assert_eq!(*script.reads.borrow(), 0, "no read before the channel of requests is live");
        states.send_replace(LIVE);
        script.until_reads(1).await;
        lost_and_back(&states).await;
        script.until_reads(2).await;
        assert_eq!(states.borrow().cluster, 0, "the same cluster");
        lost_and_back(&states).await;
        script.until_reads(3).await;
        script.until_reads(4).await;
        let mut changed = states.subscribe();
        let new = timeout(Duration::from_secs(1), changed.wait_for(|states| states.cluster == 1)).await;
        assert!(new.is_ok(), "an other cluster");
        assert_eq!(states.borrow().requests, Path::Live, "the number changes nothing else");
    };
    tokio::select! {
        ended = follow(|| script.read(), &states) => panic!("the task ended: {ended:?}"),
        () = steps => {}
    }
}

/// A user who cannot read `kube-system` gets no check and no more reads.
#[tokio::test(start_paused = true)]
async fn a_refused_read_ends_the_reads() {
    let script = Script::new(vec![Err(CoreError::answered(403, "forbidden"))]);
    let states = watch::Sender::new(LIVE);
    let steps = async {
        script.until_reads(1).await;
        lost_and_back(&states).await;
        sleep(Duration::from_secs(60)).await;
        assert_eq!((*script.reads.borrow(), states.borrow().cluster), (1, 0));
    };
    tokio::select! {
        ended = follow(|| script.read(), &states) => panic!("the task ended: {ended:?}"),
        () = steps => {}
    }
}
