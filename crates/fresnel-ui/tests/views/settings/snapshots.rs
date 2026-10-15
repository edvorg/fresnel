//! PNG files of the settings view, the overlay of the keys, a read-only cluster and the
//! confirm rule `type-name`, for a look by a person.

use std::path::PathBuf;
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::config::{FileRead, FileState, Problem};
use k8s_openapi::api::apps::v1::Deployment;

use super::with_config;
use crate::data::{live, pods, scene};
use crate::fake::{open_page, view, window_with};
use crate::snapshots::{SCALE, save_clean};
use crate::workloads::scene::{allowed, open};

const FILE: &str = "version = 1\n[appearance]\ntime_format = \"12h\"\n[table]\nrow_height = 28\n[safety]\nconfirm = \"type-name\"\n\
                    [clusters.ams-prod]\nalias = \"AMS\"\ncolor = \"#f0883e\"\nread_only = true\n\
                    default_namespace = \"scene\"\n[groups]\neurope = [\"ams-prod\", \"fra-prod\"]\n";

fn settings_view(theme: Option<&str>) -> fresnel_core::app::CoreView {
    let text = match theme {
        Some(theme) => {
            FILE.replace("time_format = \"12h\"\n", &format!("time_format = \"12h\"\ntheme = \"{theme}\"\n"))
        }
        None => FILE.to_owned(),
    };
    let mut view = with_config(view(vec![live("ams-prod", &pods()), live("fra-prod", &pods())]), &text);
    let problems = vec![
        Problem {
            line: 4,
            column: 14,
            text: "`table.row_height` = 50 is above its highest value; the app uses 40 pt".to_owned(),
        },
        Problem { line: 9, column: 1, text: "unknown key `zoom` in `appearance`; the app ignores it".to_owned() },
    ];
    let path = PathBuf::from("/Users/tester/Library/Application Support/fresnel/config.toml");
    view.config_file = Arc::new(FileState { path, read: FileRead::Read { problems, used: true }, save: None });
    view
}

#[test]
fn the_settings_view_in_the_dark_and_the_light_theme() {
    for (theme, name) in [(None, "m8-settings-dark"), (Some("light"), "m8-settings-light")] {
        let mut window = window_with(settings_view(theme), egui::Theme::Dark, SCALE);
        open_page(&mut window, "Settings");
        save_clean(&mut window, name);
    }
}

#[test]
fn the_overlay_of_the_keys() {
    let mut window = window_with(view(vec![live("ams-prod", &pods())]), egui::Theme::Dark, SCALE);
    window.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Questionmark);
    window.run();
    save_clean(&mut window, "m8-keys-overlay");
}

#[test]
fn a_read_only_cluster_and_the_confirm_rule_type_name() {
    let deployments = scene::<Deployment>();
    let clusters = vec![allowed("ams-prod", &deployments), allowed("fra-prod", &deployments)];
    let text = "version = 1\n[clusters.ams-prod]\nalias = \"AMS\"\nread_only = true\n[clusters.fra-prod]\nconfirm = \"type-name\"\n";
    let mut window = window_with(with_config(view(clusters), text), egui::Theme::Dark, SCALE);
    open(&mut window, "Deployments", "web");
    save_clean(&mut window, "m8-read-only-cluster");
    let rows = window.query_all_by_role_and_label(Role::Label, "fra-prod").count();
    assert!(rows > 0, "the row of fra-prod");
    window.query_all_by_role_and_label(Role::Label, "web").last().expect("the row of fra-prod").click();
    window.run();
    window.get_by_role_and_label(Role::Button, "Restart…").click();
    window.run();
    save_clean(&mut window, "m8-confirm-type-name");
}
