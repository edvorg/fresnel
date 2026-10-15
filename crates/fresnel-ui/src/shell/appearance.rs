//! The settings that act on the window at once (plan section 12.5): the theme, the time
//! zone and the form of a time of day, the colors of the clusters, and the default
//! namespaces of the filter at the start.
//!
//! A setting that the file does not set changes nothing: the window keeps the theme of the
//! system and the clock its zone. A view test thus keeps its fixed theme and clock.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{Color32, Context, ThemePreference};
use fresnel_core::app::{CoreView, Startup};
use fresnel_core::config::table::Setting;
use fresnel_core::config::{Config, FileRead, Rgb, Theme};
use jiff::tz::TimeZone;

use crate::format::Clock;
use crate::theme::ClusterColors;

/// What the window took from the settings, so a frame changes only what changed.
#[derive(Debug, Default)]
pub(super) struct Appearance {
    theme: Option<Theme>,
    /// The time zone and the form of the clock, as the file gives them.
    time: Option<(String, String)>,
    /// True after the filter got the default namespaces of the settings.
    namespaces: bool,
}

impl Appearance {
    /// Gives the settings of this frame to the window.
    pub(super) fn apply(
        &mut self,
        ctx: &Context,
        view: &CoreView,
        (clock, colors, namespaces): (&mut Clock, &mut ClusterColors, &mut BTreeSet<Arc<str>>),
    ) {
        let config = &view.config;
        let theme = config.set(Setting::Theme).map(|_| config.theme());
        if theme != self.theme {
            ctx.set_theme(match theme {
                Some(Theme::Dark) => ThemePreference::Dark,
                Some(Theme::Light) => ThemePreference::Light,
                Some(Theme::System) | None => ThemePreference::System,
            });
            self.theme = theme;
        }
        let set = config.set(Setting::TimeZone).is_some() || config.set(Setting::TimeFormat).is_some();
        let time =
            set.then(|| (config.text(Setting::TimeZone).to_owned(), config.text(Setting::TimeFormat).to_owned()));
        if time != self.time {
            let (zone, form) = time.clone().unwrap_or_else(|| ("local".to_owned(), "24h".to_owned()));
            clock.set(zone_of(&zone), form == "12h");
            self.time = time;
        }
        colors.choose(view.clusters.iter().filter_map(|row| Some((row.id.clone(), color(config.color(&row.id)?)))));
        self.default_namespaces(view, namespaces);
    }

    /// The filter of the first frame after the read of the files gets the default
    /// namespace of each selected cluster, unless the user chose namespaces before.
    fn default_namespaces(&mut self, view: &CoreView, chosen: &mut BTreeSet<Arc<str>>) {
        if self.namespaces || view.startup != Startup::Ready || view.config_file.read == FileRead::NotYet {
            return;
        }
        self.namespaces = true;
        if chosen.is_empty() {
            chosen.extend(defaults(&view.config, view));
        }
    }
}

fn defaults<'a>(config: &'a Config, view: &'a CoreView) -> impl Iterator<Item = Arc<str>> + 'a {
    let selected = view.clusters.iter().filter(|row| row.selected);
    selected.filter_map(|row| config.options(&row.id)?.default_namespace.as_deref().map(Arc::from))
}

/// The time zone of a text of the settings: "local", "UTC" or a name of the database. The
/// reader of the file refused each other text, so the fallback is for a database that
/// changed since.
fn zone_of(text: &str) -> TimeZone {
    match text {
        "local" => TimeZone::system(),
        "UTC" => TimeZone::UTC,
        name => TimeZone::get(name).unwrap_or_else(|_| TimeZone::system()),
    }
}

fn color(Rgb([red, green, blue]): Rgb) -> Color32 {
    Color32::from_rgb(red, green, blue)
}
