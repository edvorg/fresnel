//! The values of the settings: what the file sets, and the default of the table for the rest.

use std::time::Duration;

use super::Config;
use super::clusters::{ClusterOptions, ConfirmRule, Rgb};
use super::table::{self, Form, Setting, Unit};
use crate::cluster::ClusterId;

/// The value of one setting of a fixed group, inside its bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Number(u64),
    /// A choice of the list of its setting, or a text.
    Text(String),
    Flag(bool),
    List(Vec<String>),
}

/// The theme of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    /// The theme of macOS, also after a change of it.
    System,
    Dark,
    Light,
}

impl Config {
    /// The value that the file sets. `None` when the setting has its default.
    pub fn set(&self, id: Setting) -> Option<&Value> {
        self.values.get(&id)
    }

    /// A number of the file, or its default.
    pub fn number(&self, id: Setting) -> u64 {
        match (self.set(id), table::spec(id).map(|spec| spec.form)) {
            (Some(Value::Number(value)), _) => *value,
            (_, Some(Form::Number(number))) => number.default,
            _ => 0,
        }
    }

    /// A number of seconds or milliseconds as a time. Another unit gives seconds.
    pub fn duration(&self, id: Setting) -> Duration {
        let value = self.number(id);
        match table::spec(id).map(|spec| spec.form) {
            Some(Form::Number(number)) if number.unit == Unit::Millis => Duration::from_millis(value),
            _ => Duration::from_secs(value),
        }
    }

    /// A choice or a text of the file, or its default.
    pub fn text(&self, id: Setting) -> &str {
        match (self.set(id), table::spec(id).map(|spec| spec.form)) {
            (Some(Value::Text(text)), _) => text,
            (_, Some(Form::Choice { default, .. } | Form::Text(default))) => default,
            _ => "",
        }
    }

    /// A flag of the file, or its default.
    pub fn flag(&self, id: Setting) -> bool {
        match (self.set(id), table::spec(id).map(|spec| spec.form)) {
            (Some(Value::Flag(value)), _) => *value,
            (_, Some(Form::Flag(default))) => default,
            _ => false,
        }
    }

    /// A list of the file, or its default.
    pub fn list(&self, id: Setting) -> Vec<String> {
        match (self.set(id), table::spec(id).map(|spec| spec.form)) {
            (Some(Value::List(items)), _) => items.clone(),
            (_, Some(Form::List(default))) => default.iter().map(|item| (*item).to_owned()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn theme(&self) -> Theme {
        match self.text(Setting::Theme) {
            "dark" => Theme::Dark,
            "light" => Theme::Light,
            _ => Theme::System,
        }
    }

    /// The options of a cluster that the file names.
    pub fn options(&self, cluster: &ClusterId) -> Option<&ClusterOptions> {
        self.clusters.get(cluster.as_str())
    }

    /// The short name of a cluster in the window, if the file gives one.
    pub fn alias(&self, cluster: &ClusterId) -> Option<&str> {
        self.options(cluster).and_then(|options| options.alias.as_deref())
    }

    /// The color that the file gives a cluster.
    pub fn color(&self, cluster: &ClusterId) -> Option<Rgb> {
        self.options(cluster).and_then(|options| options.color)
    }

    /// True when the app makes no change in the cluster: its own option, or else the one
    /// of `[safety]`.
    pub fn read_only(&self, cluster: &ClusterId) -> bool {
        self.options(cluster).and_then(|options| options.read_only).unwrap_or_else(|| self.flag(Setting::ReadOnly))
    }

    /// Why the app makes no change in a cluster, if it makes none.
    pub fn refusal(&self, cluster: &ClusterId) -> Option<String> {
        self.read_only(cluster)
            .then(|| format!("{cluster} is read-only in the settings: the app makes no change in it."))
    }

    /// The confirm rule of a cluster: its own option, or else the one of `[safety]`.
    pub fn confirm_rule(&self, cluster: &ClusterId) -> ConfirmRule {
        let own = self.options(cluster).and_then(|options| options.confirm);
        own.or_else(|| ConfirmRule::named(self.text(Setting::Confirm))).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::table::{FIXED_FOLDERS, TABLE};

    #[test]
    fn a_setting_of_the_file_wins_and_each_other_one_has_the_default_of_the_table() {
        let mut config = Config::default();
        assert_eq!(config.number(Setting::MetricsPeriod), 15);
        assert_eq!(config.duration(Setting::HedgeMin), Duration::from_millis(150));
        assert_eq!((config.text(Setting::Theme), config.theme()), ("system", Theme::System));
        assert_eq!(config.list(Setting::FixedFolders), FIXED_FOLDERS, "of the system: none on Windows");
        assert!(config.flag(Setting::NotifyFailed) && !config.flag(Setting::ReadOnly));
        assert!(TABLE.iter().all(|spec| config.set(spec.id).is_none()));

        config.values.insert(Setting::MetricsPeriod, Value::Number(30));
        config.values.insert(Setting::Theme, Value::Text("light".to_owned()));
        assert_eq!((config.duration(Setting::MetricsPeriod), config.theme()), (Duration::from_secs(30), Theme::Light));
    }

    #[test]
    fn the_own_option_of_a_cluster_wins_over_the_safety_group() {
        let (ams, fra) = (ClusterId::new("ams"), ClusterId::new("fra"));
        let mut config = Config::default();
        assert_eq!((config.read_only(&ams), config.refusal(&ams)), (false, None));
        config.values.insert(Setting::ReadOnly, Value::Flag(true));
        config.values.insert(Setting::Confirm, Value::Text("type-name".to_owned()));
        let own = ClusterOptions { read_only: Some(false), confirm: Some(ConfirmRule::Dialog), ..Default::default() };
        config.clusters.insert("fra".to_owned(), own);
        assert!(config.read_only(&ams) && !config.read_only(&fra));
        assert_eq!(
            config.refusal(&ams).as_deref(),
            Some("ams is read-only in the settings: the app makes no change in it.")
        );
        assert_eq!(
            (config.confirm_rule(&ams), config.confirm_rule(&fra)),
            (ConfirmRule::TypeName, ConfirmRule::Dialog)
        );
    }
}
