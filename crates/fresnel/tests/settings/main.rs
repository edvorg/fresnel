//! The settings view and the notices on the real core, with no window (plan sections 11.6
//! and 12.5): a save writes the file with its comments, the window takes the new settings,
//! and a change of the file in an editor reaches the window. A file that does not parse
//! leaves the last settings. A failed operation and a cluster that stays lost while the
//! window is hidden give a notice.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/window.rs"]
mod window;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{Command, Notice};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::ops::{Operation, Order, Target};
use fresnel_core::store::ObjectKey;
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

/// Tells the window that it lost the focus to some other app, or that it has it again.
fn set_hidden(window: &mut Window, hidden: bool) {
    let viewport = window.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default();
    viewport.focused = Some(!hidden);
    window.run();
}

#[test]
fn a_failed_operation_and_a_cluster_that_stays_lost_while_the_window_is_hidden_give_a_notice() {
    let runtime: Runtime = Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("a runtime");
    let home = Home::new("notices", &["kubeconfig.yaml"]).expect("a home folder");
    // A cluster whose server refuses each connect: it is lost from its first attempts. The
    // fake server gives only its certificate.
    let server = runtime.block_on(FakeApiServer::start()).expect("a server");
    let authority = common::authority(&server);
    let text = format!(
        "current-context: gone\nclusters:\n- {{name: gone, cluster: {{server: 'https://127.0.0.1:1', \
         certificate-authority-data: {authority}}}}}\ncontexts:\n- {{name: gone, context: {{cluster: gone}}}}\n"
    );
    home.write("work/kubeconfig.yaml", &text).expect("the kubeconfig");
    home.write(CONFIG, "version = 1\n[notifications]\nlost_after_secs = 10\n").expect("the settings file");
    let notices = Arc::new(Mutex::new(Vec::<Notice>::new()));
    let core = core(&runtime, &home);
    core.set_notifier({
        let notices = Arc::clone(&notices);
        Arc::new(move |notice| notices.lock().expect("the notices").push(notice))
    });
    let mut window = window(core, egui::vec2(1280.0, 800.0));
    let titles = || notices.lock().expect("the notices").iter().map(|notice| notice.title.clone()).collect::<Vec<_>>();
    until(&mut window, "the lost cluster", |window| shows(window, "gone: no connection"));

    // A job of a cluster that is not connected ends as not sent: a failed operation.
    let target = Target {
        cluster: ClusterId::new("elsewhere"),
        kind: Kind::Pod,
        key: ObjectKey::new("shop", "web"),
        uid: "u".into(),
    };
    let order = Order::new(target, Operation::Evict).expect("an order");
    window.state().core.send(Command::Run(vec![order]));
    until(&mut window, "the notice of the failed operation", |_| titles() == ["Evict failed in elsewhere"]);

    // The window shows the lost cluster: after the time of the settings, no notice.
    let shown = std::time::Instant::now();
    while shown.elapsed() < Duration::from_secs(11) {
        window.run();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(titles(), ["Evict failed in elsewhere"], "no notice while the window shows the state");
    set_hidden(&mut window, true);
    until(&mut window, "the notice of the lost cluster", |_| titles().len() == 2);
    let lost = titles().pop().expect("a title");
    assert!(lost.starts_with("gone is lost for "), "{lost}");
    let body = notices.lock().expect("the notices").last().map(|notice| notice.body.clone()).unwrap_or_default();
    assert!(body.contains("Connection refused"), "the reason: {body}");
    set_hidden(&mut window, false);
    stop_with_no_ended_task(&window);
}
