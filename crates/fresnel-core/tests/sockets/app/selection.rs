//! The selection of clusters in the state file: one run gives it to the next run.

use std::time::Duration;

use fresnel_core::app::{CoreSettings, CoreView, Health, Startup};
use fresnel_core::journal::Level;
use fresnel_testkit::{FakeApiServer, Home};

use super::{App, STATE_FILE, home, states, write_kubeconfig};
use crate::common::in_time;

/// The selection in the state file. `None` with no file.
fn saved(home: &Home) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(home.path(STATE_FILE)).ok()?;
    let document: serde_json::Value = serde_json::from_str(&text).expect("the state file is JSON");
    assert_eq!(document["version"], 1, "{text}");
    let clusters = document["selected_clusters"].as_array().expect("the selection");
    Some(clusters.iter().map(|cluster| cluster.as_str().expect("a name").to_owned()).collect())
}

/// Waits until the state file has this selection. The core writes the file on a thread for
/// blocking calls and gives no signal for it, so the test reads the file again.
async fn until_saved(home: &Home, clusters: &[&str]) {
    let written = async {
        while saved(home).as_deref() != Some(&clusters.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>()[..]) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    in_time(&format!("the state file has the selection {clusters:?}"), written).await;
}

async fn three_servers() -> [FakeApiServer; 3] {
    let server = || async { FakeApiServer::start().await.expect("a server") };
    [server().await, server().await, server().await]
}

#[tokio::test(flavor = "multi_thread")]
async fn the_next_run_starts_with_the_selection_of_the_last_run() {
    let [a, b, c] = three_servers().await;
    let home = home("selection");
    write_kubeconfig(&home, "a", &[("a", a.addr(), &a, false), ("b", b.addr(), &b, false), ("c", c.addr(), &c, false)]);

    // The first run has no state file, so it selects the cluster of `kubectl`.
    let first = App::started(&home, CoreSettings::default());
    first.until_health("a", Health::Live).await;
    assert_eq!(saved(&home), None, "the core writes no file before the user changes the selection");
    for (cluster, selected) in [("b", true), ("c", true), ("a", false)] {
        first.select(cluster, selected);
    }
    until_saved(&home, &["b", "c"]).await;
    first.until_health("c", Health::Live).await;
    drop(first);
    for server in [&a, &b, &c] {
        in_time("each connection closes", server.wait_for(|counters| counters.open_connections == 0)).await;
    }

    let second = App::started(&home, CoreSettings::default());
    second.until_health("b", Health::Live).await;
    let view = second.until_health("c", Health::Live).await;
    assert_eq!(states(&view), [("a", false, Health::Off), ("b", true, Health::Live), ("c", true, Health::Live)]);
    assert_eq!(a.counters().connections, 3, "the second run did not connect the cluster of `kubectl`");
    assert!(
        second.texts_of("app").contains(&"the selection of the last run: b, c".to_owned()),
        "{:?}",
        second.journal()
    );

    // A selection with no cluster is a selection too: the next run connects nothing.
    second.select("b", false);
    second.select("c", false);
    until_saved(&home, &[]).await;
    drop(second);
    let third = App::started(&home, CoreSettings::default());
    let view = third.until("the start is complete", |view| view.startup == Startup::Ready).await;
    assert_eq!(states(&view), [("a", false, Health::Off), ("b", false, Health::Off), ("c", false, Health::Off)]);
    assert!(third.texts_of("app").contains(&"the selection of the last run: no cluster".to_owned()));
    assert_eq!((a.counters().connections, b.counters().connections), (3, 6), "the third run connected no cluster");
}

/// Plan section 12.5: the app does not write over a file that it cannot read.
#[tokio::test(flavor = "multi_thread")]
async fn a_state_file_that_does_not_load_stays_as_it_is_and_the_start_selects_the_cluster_of_kubectl() {
    let [a, b, _c] = three_servers().await;
    let cases = [
        ("{\"version\": 1,\n \"selected_clusters\": [\"b\"\n", "line 3, column 0 does not fit the format of version 1"),
        (r#"{"version": 2, "windows": [{"selected": ["b"]}]}"#, "the format has the version 2, and this app reads 1"),
    ];
    for (text, problem) in cases {
        let home = home("selection-unread");
        write_kubeconfig(&home, "a", &[("a", a.addr(), &a, false), ("b", b.addr(), &b, false)]);
        home.write(STATE_FILE, text).expect("the state file");
        let app = App::started(&home, CoreSettings::default());
        let view = app.until_health("a", Health::Live).await;
        assert_eq!(states(&view), [("a", true, Health::Live), ("b", false, Health::Off)]);
        let file = home.path(STATE_FILE).display().to_string();
        let warning =
            format!("{file}: {problem}; the app does not write over this file, so it does not save the selection");
        assert!(app.journal().contains(&(Level::Warning, "app".to_owned(), warning)), "{:#?}", app.journal());

        // Two changes of the selection. After the second one, the time of the write for
        // the first one is over.
        app.select("b", true);
        app.until_health("b", Health::Live).await;
        app.select("b", false);
        app.until_health("b", Health::Off).await;
        assert_eq!(home.read(STATE_FILE), text, "the core wrote over a file that it did not read");
    }
}

/// A kubeconfig file can be absent for one run. The selection of its clusters stays.
#[tokio::test(flavor = "multi_thread")]
async fn a_saved_cluster_that_no_kubeconfig_file_has_stays_in_the_state_file() {
    let [a, b, _c] = three_servers().await;
    let home = home("selection-absent");
    write_kubeconfig(&home, "b", &[("a", a.addr(), &a, false), ("b", b.addr(), &b, false)]);
    home.write(STATE_FILE, r#"{"version": 1, "selected_clusters": ["a", "other-file"]}"#).expect("the state file");
    let app = App::started(&home, CoreSettings::default());
    let view = app.until_health("a", Health::Live).await;
    assert_eq!(states(&view), [("a", true, Health::Live), ("b", false, Health::Off)], "the file wins over `kubectl`");
    let note = "no kubeconfig file has these clusters of the last selection: other-file; they stay in the state file";
    assert!(app.journal().contains(&(Level::Warning, "app".to_owned(), note.to_owned())), "{:#?}", app.journal());

    app.select("b", true);
    until_saved(&home, &["a", "b", "other-file"]).await;
    app.select("a", false);
    until_saved(&home, &["b", "other-file"]).await;
}

/// A write of the state file that fails, as on a disk with no space: the window shows the
/// warning, the selection works in this run, and the next change writes the selection.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_that_fails_is_a_warning_and_the_next_change_saves_the_selection() {
    use std::os::unix::fs::PermissionsExt as _;

    let [a, b, _c] = three_servers().await;
    let home = home("selection-no-write");
    write_kubeconfig(&home, "a", &[("a", a.addr(), &a, false), ("b", b.addr(), &b, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until_health("a", Health::Live).await;

    // The folder takes no new file.
    let folder = home.path(STATE_FILE).parent().expect("the folder of the state file").to_owned();
    let permit = |mode| std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).expect("the mode");
    permit(0o500);
    app.select("a", false);
    let not_saved = |view: &CoreView| {
        let warnings =
            view.journal.entries.iter().filter(|entry| entry.text.starts_with("the selection is not saved: "));
        warnings.map(|entry| (entry.level, entry.source.clone())).collect::<Vec<_>>()
    };
    let view = app.until("the warning of the failed write", |view| !not_saved(view).is_empty()).await;
    permit(0o700);
    assert_eq!(not_saved(&view), [(Level::Warning, "app".to_owned())]);
    assert_eq!(states(&view), [("a", false, Health::Off), ("b", false, Health::Off)], "the selection works");
    assert_eq!(saved(&home), None);

    app.select("b", true);
    until_saved(&home, &["b"]).await;
    let view = app.until_health("b", Health::Live).await;
    assert_eq!(not_saved(&view).len(), 1, "one failed write gives one warning");
}
