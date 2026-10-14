//! PNG files of the views in the dark theme, for a look by a person. The files of the Pods
//! view come from `pods/snapshots.rs`.
//!
//! The files go to the folder of `FRESNEL_SNAPSHOT_DIR`, or to the test folder of the
//! build. A test checks only that a frame has the size and is not blank: a comparison of
//! each pixel would fail for each change of a font or of a GPU driver.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use fresnel_core::app::{CoreView, Startup};
use fresnel_core::cluster::LinkState;
use fresnel_core::journal::Level;
use fresnel_core::net::credential::{CredentialFailure, CredentialState, PluginError, PluginFailure, RunReason};
use fresnel_core::net::{ChannelState, ChannelStatus, OutageStatus};

use crate::fake::{
    SECOND, Window, channel, connected, error, journal, live_status, new_channel, now, off, open_page, row, status,
    valid, view, window_with,
};

/// The scale of a Retina display.
pub(crate) const SCALE: f32 = 2.0;

fn folder() -> PathBuf {
    std::env::var_os("FRESNEL_SNAPSHOT_DIR").map_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")), PathBuf::from)
}

pub(crate) fn save(window: &mut Window, name: &str) {
    let image = window.render().expect("a GPU adapter draws the frame");
    assert_eq!((image.width(), image.height()), (2560, 1600));
    let colors: HashSet<_> = image.pixels().collect();
    assert!(colors.len() > 20, "{name} has only {} colors", colors.len());
    let folder = folder();
    std::fs::create_dir_all(&folder).expect("the snapshot folder");
    image.save(folder.join(format!("{name}.png"))).expect("the PNG file");
}

/// The four clusters of a session with a network fault and a plugin fault.
fn session() -> CoreView {
    let mut live = live_status();
    live.watches.channels[0].streams_open = 30;
    live.requests.channels[0].ping_time = Some(Duration::from_millis(14));

    let first_run =
        CredentialState::Running { reason: RunReason::Start, since: now() - 2 * SECOND, last_failure: None };
    let connecting = [
        new_channel(ChannelState::Connecting { attempt: 1 }),
        new_channel(ChannelState::Verifying),
        new_channel(ChannelState::Connecting { attempt: 1 }),
    ];

    let lost = |failures, pause: u32, attempts, error: &str| ChannelStatus {
        since: now() - SECOND,
        last_frame_age: Some(48 * SECOND),
        connect_attempts: attempts,
        last_error: Some(error.to_owned()),
        ..channel(ChannelState::Backoff { failures, pause: pause * SECOND })
    };
    // An outage of 12 minutes: the channels connect in turns, one attempt each 5 s.
    let no_connection = [
        lost(97, 4, 97, "connect: no answer in 3s"),
        lost(96, 9, 96, "connect: no answer in 3s"),
        lost(95, 14, 95, "connect: no answer in 3s"),
    ];
    let mut long_outage = status(no_connection, Some(valid()));
    long_outage.outage = Some(OutageStatus { since: now() - 720 * SECOND, turn: Some(5 * SECOND) });

    let detail =
        "Error: Unable to initialize DigitalOcean API client: access token is required. (hint: run 'doctl auth init')";
    let plugin = PluginError::new("doctl", PluginFailure::Exit("exit status: 1".to_owned()), detail.to_owned());
    let failed = CredentialState::Failed {
        failure: CredentialFailure::Plugin(plugin),
        failures: 2,
        next_attempt: now() + 8 * SECOND,
    };
    let no_token = [(); 3].map(|()| new_channel(ChannelState::WaitsForToken));

    let mut with_journal = view(vec![
        connected("ams-prod", live),
        connected("fra-stage", status(connecting, Some(first_run))),
        connected("sgp-prod", long_outage),
        connected("nyc-prod", status(no_token, Some(failed))),
        row(
            "lab",
            LinkState::Failed(error(
                "the cluster entry `lab` has no certificate authority; the app does not support that yet",
            )),
        ),
        off("minikube"),
    ]);
    with_journal.journal = journal(
        &[
            (Level::Info, "app", "the login shell /bin/zsh gave its environment in 41 ms"),
            (Level::Info, "app", "read 6 contexts from 1 kubeconfig file"),
            (Level::Info, "ams-prod", "the channels connect"),
            (Level::Info, "sgp-prod", "the channels connect"),
            (Level::Info, "ams-prod", "live: a request got an answer on each channel"),
            (Level::Info, "sgp-prod", "live: a request got an answer on each channel"),
            (Level::Error, "nyc-prod", "no credential (credential plugin `doctl`: stopped with exit status: 1)"),
            (
                Level::Error,
                "lab",
                "the connect did not start: the cluster entry `lab` has no certificate authority; the app does not support that yet",
            ),
            (Level::Info, "app", "the network changed: each cluster tests its connections and ends its retry delays"),
            (
                Level::Warning,
                "sgp-prod",
                "a connection was lost (watches: the server sent no ping answer in 5s); it connects again",
            ),
            (Level::Warning, "sgp-prod", "no connection (watches: connect: no answer in 10s); the channel tries again"),
            (Level::Info, "fra-stage", "the channels connect"),
        ],
        0,
    );
    with_journal
}

#[test]
fn the_connections_view_in_the_dark_theme() {
    let mut window = window_with(session(), egui::Theme::Dark, SCALE);
    open_page(&mut window, "Connections");
    window.remove_cursor();
    window.run();
    save(&mut window, "connections");
}

/// The cards of the clusters with a fault come first, so that the frame shows all of them.
#[test]
fn the_cards_of_clusters_with_a_fault_in_the_dark_theme() {
    let mut faults = session();
    faults.clusters.retain(|row| row.id.as_str() != "ams-prod");
    faults.clusters.reverse();
    let mut window = window_with(faults, egui::Theme::Dark, SCALE);
    open_page(&mut window, "Connections");
    window.remove_cursor();
    window.run();
    save(&mut window, "connections-faults");
}

#[test]
fn the_window_at_its_start_in_the_dark_theme() {
    let mut starting = view(Vec::new());
    starting.startup = Startup::Environment;
    let mut window = window_with(starting, egui::Theme::Dark, SCALE);
    save(&mut window, "start");
}

#[test]
fn the_connections_view_in_the_light_theme() {
    let mut window = window_with(session(), egui::Theme::Light, SCALE);
    open_page(&mut window, "Connections");
    window.remove_cursor();
    window.run();
    save(&mut window, "connections-light");
}
