//! The settings view on the real core, with no window (plan section 12.5): a save writes
//! the file with its comments, the window takes the new settings, and a change of the file
//! in an editor reaches the window. A file that does not parse leaves the last settings.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/window.rs"]
mod window;

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_testkit::{FakeApiServer, Home};
use tokio::runtime::{Builder, Runtime};

use window::{Window, core, shows, stop_with_no_ended_task, until, window};

const CONFIG: &str = "state/config.toml";
const FIRST: &str = "# my settings\nversion = 1\n[kubeconfig]\nwatch_secs = 1 # fast\n";

fn home(server: &FakeApiServer) -> Home {
    let home = Home::new("settings-view", &["kubeconfig.yaml"]).expect("a home folder");
    let (address, authority) = (server.addr(), common::authority(server));
    let text = format!(
        "current-context: ams\nclusters:\n- {{name: ams, cluster: {{server: 'https://{address}', \
         certificate-authority-data: {authority}}}}}\ncontexts:\n- {{name: ams, context: {{cluster: ams}}}}\n"
    );
    home.write("work/kubeconfig.yaml", &text).expect("the kubeconfig");
    home.write(CONFIG, FIRST).expect("the settings file");
    home
}

/// Replaces the text of a field of the settings view, as a user who selects it and types.
fn retype(window: &mut Window, field: &str, text: &str) {
    window.get_by_role_and_label(Role::TextInput, field).scroll_to_me();
    window.run();
    window.get_by_role_and_label(Role::TextInput, field).click();
    window.run();
    window.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    window.get_by_role_and_label(Role::TextInput, field).type_text(text);
    window.run();
}

fn chip(window: &Window, name: &str) -> bool {
    window.query_by_role_and_label(Role::CheckBox, name).is_some()
}

#[test]
fn a_save_of_the_settings_view_keeps_the_comments_and_the_window_follows_the_file() {
    let runtime: Runtime = Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("a runtime");
    let server = runtime.block_on(FakeApiServer::start()).expect("a server");
    let home = home(&server);
    let mut window = window(core(&runtime, &home), egui::vec2(1280.0, 900.0));
    until(&mut window, "the live cluster", |window| shows(window, "ams: live"));

    window.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
    until(&mut window, "the settings view", |window| shows(window, "Read with no problem."));
    retype(&mut window, "Height of a row", "30");
    window.get_by_role_and_label(Role::CheckBox, "Each cluster is read-only").click();
    window.run();
    window.get_by_label("Save the settings").click();
    until(&mut window, "the save", |window| shows(window, "Saved into the file."));
    assert_eq!(
        home.read(CONFIG),
        format!("{FIRST}\n[safety]\nread_only = true\n\n[table]\nrow_height = 30\n"),
        "each comment stays"
    );
    until(&mut window, "the read-only mark of the cluster", |window| chip(window, "ams (read-only)"));
    let value = window.get_by_role_and_label(Role::TextInput, "Height of a row").value();
    assert_eq!(value.as_deref(), Some("30"), "the view shows the file");

    // A change in an editor reaches the window, with no save of the view.
    home.write(CONFIG, &format!("{FIRST}[safety]\nread_only = true\n[appearance]\ntheme = \"light\"\n"))
        .expect("a new file");
    until(&mut window, "the light theme of the file", |window| window.ctx.theme() == egui::Theme::Light);
    // A mistake in the editor: the view shows its place, saves nothing, and the cluster
    // stays read-only, as the last good read says.
    home.write(CONFIG, &format!("{FIRST}[safety\nread_only = false\n")).expect("a broken file");
    until(&mut window, "the place of the mistake", |window| {
        window.query_all_by_label_contains("line 5, column 8: ").next().is_some()
    });
    assert!(shows(&window, "The app uses no setting of the file:"));
    assert!(window.get_by_label("Save the settings").accesskit_node().is_disabled());
    assert!(chip(&window, "ams (read-only)") && window.ctx.theme() == egui::Theme::Light, "the last settings");
    stop_with_no_ended_task(&window);
}
