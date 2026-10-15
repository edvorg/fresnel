//! The saves of a log view: the command of each one, the state of the last one in the tab,
//! and its cancel.

use std::path::PathBuf;
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::CoreError;
use fresnel_core::app::Command;
use fresnel_core::logs::{LogSave, SavePart, SaveState, SaveView, SourceState};
use fresnel_core::net::ErrorClass;

use super::{log_view, opened, show, source};
use crate::data::held;
use crate::fake::{Window, shows};

const WEB: &str = "web-6888b778b6-75md7";

/// Gives the cluster of the view this save.
fn saved(window: &mut Window, save: SaveView) {
    let mut view = window.state().view.clone();
    if let Some(data) = view.clusters.first_mut().and_then(|row| row.data.as_mut()) {
        data.saves = Arc::from([held(save)]);
    }
    window.state_mut().view = view;
    window.run();
}

fn saves(window: &Window) -> Vec<Command> {
    let sent = window.state().sent.iter();
    sent.filter(|command| matches!(command, Command::SaveLog(_) | Command::CancelSave { .. })).cloned().collect()
}

#[test]
fn a_save_sends_its_command_and_the_tab_shows_its_state_and_a_cancel() {
    let (mut window, request) = opened(WEB);
    show(&mut window, vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], Vec::new())]);
    window.get_by_role_and_label(Role::Button, "Save").click();
    window.run();
    window.get_by_label("Save the full log").click();
    window.run();
    window.get_by_role_and_label(Role::CheckBox, "Times").click();
    window.run();
    window.get_by_role_and_label(Role::Button, "Save").click();
    window.run();
    window.get_by_label("Save the lines of the view").click();
    window.run();
    let full = LogSave { request: request.clone(), part: SavePart::Full, times: true };
    let buffer = LogSave { request: request.clone(), part: SavePart::Buffer, times: false };
    assert_eq!(saves(&window), [Command::SaveLog(full.clone()), Command::SaveLog(buffer)], "the times of the view");

    let file = PathBuf::from("/Users/tester/Downloads/ams-prod_scene_pod-web_full_20270115-080000.log");
    let save = |state| SaveView { number: 4, save: full.clone(), file: Some(file.clone()), state };
    saved(&mut window, save(SaveState::Writing { bytes: 3 * 1024 * 1024 }));
    assert!(shows(&window, &format!("Saving 3MiB to {}…", file.display())));
    window.get_by_label("Cancel the save").click();
    window.run();
    let cancel = Command::CancelSave { cluster: request.cluster.clone(), number: 4 };
    assert_eq!(saves(&window).last(), Some(&cancel));

    saved(
        &mut window,
        save(SaveState::Done { bytes: 2048, skipped: vec!["web-2/web: pods \"web-2\" not found".to_owned()] }),
    );
    assert!(shows(
        &window,
        &format!("Saved 2KiB to {}; no log of web-2/web: pods \"web-2\" not found", file.display())
    ));
    assert!(window.query_by_label("Cancel the save").is_none(), "a save that ended has no cancel");
    saved(
        &mut window,
        save(SaveState::Failed(CoreError::new(ErrorClass::Invalid, "the file cannot be written: no space"))),
    );
    assert!(shows(&window, "The save failed: the file cannot be written: no space"));
    saved(&mut window, save(SaveState::Cancelled));
    assert!(shows(&window, "The save stopped, and its file is gone."));
}
