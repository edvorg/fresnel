use std::sync::Mutex;
use std::sync::mpsc;

use fresnel_net::ErrorClass;
use fresnel_net::task::TaskReport;
use tokio::runtime::Handle;
use tokio::time::Instant;

use super::*;
use crate::published::Disposal;

/// The answer of the server to the read with this number, counted from 0.
type Answer = fn(usize) -> Result<Arc<str>, CoreError>;

/// The reads of a test, with the time of each one.
#[derive(Clone)]
struct Script {
    answer: Answer,
    calls: Arc<Mutex<Vec<Duration>>>,
    start: Instant,
}

impl Versions for Script {
    async fn version(&self) -> Result<Arc<str>, CoreError> {
        let read = {
            let mut calls = self.calls.lock().expect("a lock");
            calls.push(self.start.elapsed());
            calls.len() - 1
        };
        (self.answer)(read)
    }
}

struct Bed {
    reports: mpsc::Receiver<TaskReport>,
    script: Script,
    reader: Reader<ServerVersion>,
    _version: ClusterVersion,
    _disposal: Disposal,
}

impl Bed {
    fn start(answer: Answer) -> Self {
        let (sender, reports) = mpsc::channel();
        let tasks = Tasks::new(Handle::current(), move |report: TaskReport| sender.send(report).expect("a reader"));
        let disposal = Disposal::start(&tasks);
        let start = Instant::now();
        let clock = WallClock::new(move || SystemTime::UNIX_EPOCH + start.elapsed());
        let script = Script { answer, calls: Arc::default(), start };
        let parts = (disposal.bin(), Notice::new());
        let version =
            ClusterVersion::with_source(&tasks, TaskName::new("server version"), script.clone(), parts, clock);
        let reader = version.reader();
        Self { reports, script, reader, _version: version, _disposal: disposal }
    }

    /// The version after the next publish.
    async fn next(&mut self) -> ServerVersion {
        self.reader.changed().await;
        ServerVersion::clone(&self.reader.read())
    }

    fn calls(&self) -> Vec<Duration> {
        self.script.calls.lock().expect("a lock").clone()
    }
}

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn timeout() -> CoreError {
    CoreError::new(ErrorClass::Timeout, "no answer in 10 s")
}

#[tokio::test(start_paused = true)]
async fn a_failed_read_shows_its_reason_keeps_the_version_and_the_next_read_comes_after_the_retry_time() {
    let mut bed = Bed::start(|read| match read {
        0 | 3 => Err(timeout()),
        1 => Ok("v1.33.5+k3s1".into()),
        _ => Ok("v1.34.1+k3s1".into()),
    });
    assert_eq!(bed.reader.read().state, VersionState::Reading, "before the first answer");
    let failed = bed.next().await;
    assert_eq!(failed, ServerVersion { version: None, state: VersionState::Failed { error: timeout(), at: at(0) } });
    let read = bed.next().await;
    assert_eq!(read, ServerVersion { version: Some("v1.33.5+k3s1".into()), state: VersionState::Read(at(15)) });
    // An upgrade of the control plane shows at the next read.
    let next = bed.next().await;
    assert_eq!((next.version.as_deref(), next.state), (Some("v1.34.1+k3s1"), VersionState::Read(at(615))));
    let kept = bed.next().await;
    let state = VersionState::Failed { error: timeout(), at: at(1215) };
    assert_eq!(kept, ServerVersion { version: Some("v1.34.1+k3s1".into()), state }, "the version of before stays");
    assert_eq!(bed.calls(), [0, 15, 615, 1215].map(Duration::from_secs));
    assert!(bed.reports.try_recv().is_err(), "no run of the task ended");
}

/// Design rule 1: a panic gives a state, a report and a new run, which keeps the version.
#[tokio::test(start_paused = true)]
async fn a_panic_of_the_task_shows_as_its_state_and_a_new_run_reads_again() {
    let mut bed = Bed::start(|read| match read {
        1 => panic!("a defect of the read"),
        _ => Ok("v1.33.5+k3s1".into()),
    });
    assert_eq!(bed.next().await.state, VersionState::Read(at(0)));
    let failed = bed.next().await;
    let internal = VersionState::InternalError("the task panicked: a defect of the read".into());
    assert_eq!(failed, ServerVersion { version: Some("v1.33.5+k3s1".into()), state: internal });
    let report = bed.reports.recv().expect("the report of the panic");
    assert!(report.to_string().contains("a defect of the read"), "{report}");
    // The new run reads after the short pause of a restart.
    let again = bed.next().await;
    let soon = matches!(again.state, VersionState::Read(time) if time > at(600) && time < at(601));
    assert!(soon && again.version.as_deref() == Some("v1.33.5+k3s1"), "{again:?}");
}
