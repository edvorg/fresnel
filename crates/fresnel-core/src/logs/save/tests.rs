//! The saves of a view on a script of streams, into a folder of the test.

use std::path::Path;
use std::time::{Duration, SystemTime};

use fresnel_net::task::TaskReport;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::*;
use crate::cluster::ClusterId;
use crate::kinds::Kind;
use crate::logs::line::{LineKind, LogLine};
use crate::logs::request::{LogOptions, Origin};
use crate::logs::ring::Lines;
use crate::logs::testing::{Call, Script, data};
use crate::logs::view::{PodsState, SourceState, SourceView};
use crate::published::{Changes, Disposal};
use crate::store::ObjectKey;

const LIMIT: Duration = Duration::from_secs(60);
/// 2025-10-12 20:00:00 UTC.
const SECOND: i64 = 1_760_299_200;

/// A folder of the test that goes at its drop.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("fresnel-test-saves-{:08x}", fastrand::u32(..))))
    }

    /// The name and the text of each file.
    fn files(&self) -> Vec<(String, String)> {
        let Ok(entries) = std::fs::read_dir(&self.0) else {
            return Vec::new();
        };
        let mut files: Vec<(String, String)> = entries
            .map(|entry| entry.expect("an entry").path())
            .map(|path| (name(&path), std::fs::read_to_string(&path).expect("a file")))
            .collect();
        files.sort();
        files
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // A test that failed can leave the folder; the system cleans its temporary folder.
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            assert!(error.kind() == std::io::ErrorKind::NotFound || std::thread::panicking(), "{error}");
        }
    }
}

fn name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

struct Bed {
    saves: Saves,
    changes: Changes,
    calls: mpsc::UnboundedReceiver<Call>,
    script: Script,
    /// The report of the end of each task.
    reports: mpsc::UnboundedReceiver<String>,
    folder: Folder,
    _disposal: Disposal,
}

impl Drop for Bed {
    /// A save ends as a job: no panic and no error.
    fn drop(&mut self) {
        while let Ok(report) = self.reports.try_recv() {
            assert!(report.ends_with("ended; no restart") || std::thread::panicking(), "{report}");
        }
    }
}

impl Bed {
    fn new() -> Self {
        let (report, reports) = mpsc::unbounded_channel();
        let tasks = Tasks::new(Handle::current(), move |end: TaskReport| {
            report.send(end.to_string()).expect("the test reads each report");
        });
        let disposal = Disposal::start(&tasks);
        let (folder, notice) = (Folder::new(), Notice::new());
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let parts = SaveParts {
            tasks,
            folder: folder.0.clone(),
            clock: WallClock::new(move || at),
            bin: disposal.bin(),
            notice: notice.clone(),
        };
        let (script, calls) = Script::new();
        let saves = Saves::new(parts);
        Self { saves, changes: notice.changes(), calls, script, reports, folder, _disposal: disposal }
    }

    fn save(&mut self, part: SavePart, times: bool, view: &LogView) {
        let save = LogSave { request: view.request.clone(), part, times };
        self.saves.start_with(save, view, self.script.clone());
    }

    /// Waits until the save with this number has a final state.
    async fn ended(&mut self, number: u64) -> SaveView {
        let wait = async {
            loop {
                let found = self.saves.views().iter().find(|view| view.number == number).map(|view| (**view).clone());
                if let Some(view) = found.filter(|view| view.state.is_final()) {
                    return view;
                }
                self.changes.next().await;
            }
        };
        timeout(LIMIT, wait).await.expect("in time: the end of the save")
    }

    async fn call(&mut self) -> Call {
        timeout(LIMIT, self.calls.recv()).await.expect("in time").expect("an open")
    }
}

fn source(pod: &str) -> SourceView {
    SourceView {
        pod: ObjectKey::new("shop", pod),
        container: "app".into(),
        state: SourceState::Streaming,
        lines: 0,
        repeated: 0,
    }
}

fn line(seconds: i64, source: u32, text: &str, kind: LineKind) -> LogLine {
    LogLine {
        time: (SECOND + seconds) * 1_000_000_000,
        source,
        text: text.into(),
        spans: None,
        kind,
        late: false,
        cut: false,
    }
}

/// A view of a Deployment with two pods and three lines in its ring.
fn view() -> LogView {
    let lines = [
        line(0, 0, "first", LineKind::Log),
        line(1, 1, "second", LineKind::Log),
        line(1, 1, "the stream started again", LineKind::Marker),
    ];
    LogView {
        request: LogRequest {
            cluster: ClusterId::new("ams"),
            kind: Kind::Deployment,
            key: ObjectKey::new("shop", "web"),
            uid: "u".into(),
            options: LogOptions::default(),
        },
        pods: PodsState::Following,
        sources: Arc::new([source("web-1"), source("web-2")]),
        lines: Lines::new(40, lines),
        dropped: 40,
        bytes: 0,
        unstreamed: 0,
        restarts: 0,
    }
}

#[tokio::test]
async fn the_buffer_goes_into_a_new_file_with_the_prefix_of_each_source_and_a_second_file_gets_a_number() {
    let mut bed = Bed::new();
    bed.save(SavePart::Buffer, true, &view());
    let first = bed.ended(1).await;
    bed.save(SavePart::Buffer, false, &view());
    let second = bed.ended(2).await;
    let text = "[pod/web-1/app] 2025-10-12T20:00:00.000000000Z first\n\
                [pod/web-2/app] 2025-10-12T20:00:01.000000000Z second\n\
                [pod/web-2/app] 2025-10-12T20:00:01.000000000Z --- the stream started again\n";
    let plain = "[pod/web-1/app] first\n[pod/web-2/app] second\n[pod/web-2/app] --- the stream started again\n";
    let names =
        ["ams_shop_deployment-web_buffer_20270115-080000-2.log", "ams_shop_deployment-web_buffer_20270115-080000.log"];
    assert_eq!(bed.folder.files(), [(names[0].to_owned(), plain.to_owned()), (names[1].to_owned(), text.to_owned())]);
    assert_eq!(first.state, SaveState::Done { bytes: text.len() as u64, skipped: Vec::new() });
    assert_eq!(first.file, Some(bed.folder.0.join(names[1])));
    assert_eq!(second.state, SaveState::Done { bytes: plain.len() as u64, skipped: Vec::new() });
}

#[tokio::test]
async fn the_full_log_reads_each_source_with_no_follow_and_a_source_with_no_log_gets_a_line_and_a_note() {
    let mut bed = Bed::new();
    bed.save(SavePart::Full, true, &view());
    let first = bed.call().await;
    assert_eq!(
        (first.pod.name.as_ref(), first.previous, first.origin),
        ("web-1", false, Origin::Whole { times: true })
    );
    let stream = first.open();
    stream.send(data(&[(1.0, "one"), (2.0, "two")])).expect("the save reads");
    drop(stream);
    bed.call().await.refuse(CoreError::answered(404, "pods \"web-2\" not found"));
    let ended = bed.ended(1).await;
    let text = "==> [pod/web-1/app] <==\n1970-01-01T00:00:01.000000000Z one\n1970-01-01T00:00:02.000000000Z two\n\
                ==> [pod/web-2/app] <==\n--- the server gave no log: pods \"web-2\" not found\n";
    let file = bed.folder.files();
    assert_eq!(file.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>(), [text]);
    assert_eq!(
        ended.state,
        SaveState::Done { bytes: text.len() as u64, skipped: vec!["web-2/app: pods \"web-2\" not found".to_owned()] }
    );
}

#[tokio::test]
async fn a_cancel_ends_a_full_log_that_waits_for_data_and_removes_its_file() {
    let mut bed = Bed::new();
    bed.save(SavePart::Full, false, &view());
    let stream = bed.call().await.open();
    stream.send(data(&[(1.0, "one")])).expect("the save reads");
    let wait = async {
        while bed.folder.files().is_empty() {
            bed.changes.next().await;
        }
    };
    timeout(LIMIT, wait).await.expect("in time: the file");
    bed.saves.cancel(1);
    assert_eq!(bed.ended(1).await.state, SaveState::Cancelled);
    assert_eq!(bed.folder.files(), [], "the part of the log is gone");
    drop(stream);
}

#[tokio::test]
async fn a_folder_that_cannot_exist_fails_the_save_with_the_reason() {
    let mut bed = Bed::new();
    std::fs::write(&bed.folder.0, "a file in the place of the folder").expect("a file");
    bed.save(SavePart::Buffer, true, &view());
    let ended = bed.ended(1).await;
    assert!(
        matches!(&ended.state, SaveState::Failed(error) if error.to_string().starts_with("the file cannot be written"))
    );
    assert_eq!(ended.file, None);
    std::fs::remove_file(&bed.folder.0).expect("the file of the test goes");
}

/// Extreme sizes: a context and a pod with names of 253 characters. A file system takes a
/// name of 255 bytes at most: the save cuts each part of the name and makes its file.
#[tokio::test]
async fn a_save_of_an_object_with_the_longest_names_makes_its_file() {
    let mut bed = Bed::new();
    let mut long = view();
    long.request.cluster = ClusterId::new("c".repeat(253));
    long.request.key = ObjectKey::new(&"n".repeat(63), &"p".repeat(253));
    bed.save(SavePart::Buffer, false, &long);
    let ended = bed.ended(1).await;
    assert!(matches!(ended.state, SaveState::Done { .. }), "{:?}", ended.state);
    assert_eq!(bed.folder.files().len(), 1);
}
