//! A login shell that hangs, and a panic of the task of the core.

use std::time::Duration;

use fresnel_core::app::{CoreSettings, Health};
use fresnel_core::journal::{Entry, Journal, Level, Sink, TASK_SOURCE};
use fresnel_core::net::WallClock;
use fresnel_testkit::FakeApiServer;

use super::{App, home, kubeconfig_text, row, states, write_kubeconfig};
use crate::common::in_time;

/// A slow peer at the start: the profile of the login shell hangs. The core starts at the
/// time limit with the default kubeconfig file, and the view tells the problem.
#[tokio::test(flavor = "multi_thread")]
async fn a_login_shell_that_hangs_gives_a_note_and_the_core_starts_at_the_limit() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("slow-shell");
    let profile =
        format!("echo $$ > '{}'\necho text-of-the-profile >&2\nexec sleep 600\n", home.path("state/shell").display());
    for file in [".zprofile", ".bash_profile", ".profile"] {
        home.write(file, &profile).expect("a profile that hangs");
    }
    // With no environment of the shell, the app has no KUBECONFIG and reads the default file.
    std::fs::create_dir(home.path(".kube")).expect("the folder of the default file");
    let text = kubeconfig_text(&home, "slow", &[("slow", server.addr(), &server, false)]);
    home.write(".kube/config", &text).expect("the default kubeconfig file");
    let mut settings = CoreSettings::default();
    settings.env.shell_limit = Duration::from_millis(500);
    let app = App::started(&home, settings);

    let view = app.until_health("slow", Health::Live).await;
    let note = view.shell_problem.clone().expect("a note for the user");
    assert!(note.problem.ends_with(" gave no environment in 500ms"), "{}", note.problem);
    assert!(note.detail.contains("text-of-the-profile"), "the text of the profile is for the UI: {}", note.detail);
    let journal = app.journal();
    let warning = journal.iter().find(|(level, source, _)| *level == Level::Warning && source == "app");
    let (_, _, warning) = warning.unwrap_or_else(|| panic!("a warning of the app: {journal:?}"));
    assert!(warning.starts_with(&note.problem) && warning.contains("fixed folders"), "{warning}");
    assert!(journal.iter().all(|(_, _, text)| !text.contains("text-of-the-profile")), "the journal goes to a log");

    let shell: i32 = home.read("state/shell").trim().parse().expect("the process id of the shell");
    let gone = async {
        while nix::sys::signal::kill(nix::unistd::Pid::from_raw(shell), None).is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    in_time("the shell that hangs is gone", gone).await;
}

/// A defect in code that the task of the core runs: the sink panics for each entry that
/// starts with this text.
struct PanicFor(&'static str);

impl Sink for PanicFor {
    fn write(&mut self, entry: &Entry) {
        assert!(!entry.text.starts_with(self.0), "the defect of the test");
    }
}

/// Design rule 1 for the task of the core, with a second fault during the recovery: the
/// task panics in two commands in sequence. It starts again each time and loses no state.
#[tokio::test(flavor = "multi_thread")]
async fn the_task_of_the_core_starts_again_after_each_panic_and_runs_the_next_command() {
    let (a, b) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    let home = home("core-panic");
    write_kubeconfig(&home, "a", &[("a", a.addr(), &a, true), ("b", b.addr(), &b, false)]);
    let journal = Journal::with_sink(1000, WallClock::system(), PanicFor("not selected"));
    let mut app = App::with_journal(&home, CoreSettings::default(), journal);
    app.expected_task_ends = 2;
    app.start();
    app.select("b", true);
    app.until_health("a", Health::Live).await;
    app.until_health("b", Health::Live).await;

    // The entry for a cluster that leaves the selection panics. The second panic comes
    // while the commands of the user wait for the task.
    app.select("a", false);
    app.select("b", false);
    app.select("b", true);
    let view = app
        .until("b is live again", |view| b.counters().connections == 6 && row(view, "b").health() == Health::Live)
        .await;
    assert_eq!(states(&view), [("a", false, Health::Off), ("b", true, Health::Live)], "the start did not run again");
    in_time("each connection of a closes", a.wait_for(|counters| counters.open_connections == 0)).await;

    let ended: Vec<(Level, String)> = app
        .journal()
        .into_iter()
        .filter(|(_, source, _)| source == TASK_SOURCE)
        .map(|(level, _, text)| (level, text))
        .collect();
    assert_eq!(ended.len(), 2, "{ended:?}");
    for (level, text) in &ended {
        assert_eq!(*level, Level::Error);
        assert!(text.starts_with("task app core panicked: the defect of the test; restart in "), "{text}");
    }
    let loads = app.texts_of("app").iter().filter(|text| text.starts_with("read 2 contexts")).count();
    assert_eq!((loads, home.read("state/runs").as_str()), (1, "run\n"), "one load of the files and one plugin run");
}
