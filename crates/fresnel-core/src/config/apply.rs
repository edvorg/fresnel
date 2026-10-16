//! The settings of the file in the settings of the core: each value that the file sets
//! replaces the value of the program, and each other value stays.
//!
//! The program and a test give the core its settings, and a test gives short times there.
//! A file with no key of a group thus changes nothing of that group.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::Config;
use super::table::Setting as S;
use crate::app::CoreSettings;
use crate::store::EventsMode;

/// The settings of the core with the values that the file sets. `home` is the home folder
/// of the user, for a path of the file that starts with `~/`.
pub(crate) fn apply(config: &Config, base: &CoreSettings, home: Option<&Path>) -> CoreSettings {
    let mut settings = base.clone();
    let time = |id| config.set(id).map(|_| config.duration(id));
    let number = |id| config.set(id).map(|_| config.number(id));
    let net = &mut settings.net;
    for lane in [&mut net.watches, &mut net.requests, &mut net.logs] {
        put(&mut lane.keepalive_interval, time(S::Keepalive));
        put(&mut lane.keepalive_timeout, time(S::PingTimeout));
        put(&mut lane.light_period, time(S::LightRequest));
        put(&mut lane.stream_budget_percent, number(S::StreamBudget).map(size));
        put(&mut lane.max_connections, number(S::MaxConnections).map(size));
    }
    // The WebSockets of exec, attach and port forward have the ping of the channels.
    put(&mut net.upgrade.socket.ping_after, time(S::Keepalive));
    put(&mut net.upgrade.socket.answer_limit, time(S::PingTimeout));
    for lane in [&mut net.watches, &mut net.logs] {
        put(&mut lane.stream_window, number(S::StreamWindow).map(kib));
        put(&mut lane.connection_window, number(S::ConnectionWindow).map(mib));
    }
    put(&mut net.requests.stream_window, number(S::ReadWindow).map(kib));
    put(&mut net.requests.connection_window, number(S::ReadConnectionWindow).map(mib));
    put(&mut net.connect.hedge_percent, number(S::HedgePercent).map(|value| u32::try_from(value).unwrap_or(u32::MAX)));
    put(&mut net.connect.hedge_min, time(S::HedgeMin));
    put(&mut net.connect.hedge_max, time(S::HedgeMax));
    put(&mut net.connect.resolve_limit, time(S::ResolveLimit));
    put(&mut net.credential.plugin_limit, time(S::PluginLimit));
    put(&mut net.credential.request_wait, time(S::PluginWait));
    put(&mut net.limits.get, time(S::GetLimit));
    put(&mut net.limits.write, time(S::WriteLimit));
    put(&mut net.limits.list_idle, time(S::ListIdle));
    put(&mut net.limits.list_total, time(S::ListTotal));

    let stores = &mut settings.stores;
    put(&mut stores.watch.page_size, number(S::PageSize).map(|value| u32::try_from(value).unwrap_or(u32::MAX)));
    put(&mut stores.watch.lists, number(S::Lists).map(size));
    if config.set(S::WatchTimeout).is_some() || config.set(S::WatchSpread).is_some() {
        let seconds = |id| u32::try_from(config.number(id)).unwrap_or(u32::MAX);
        let shortest = seconds(S::WatchTimeout);
        stores.watch.server_timeout = shortest..=shortest.saturating_add(seconds(S::WatchSpread));
    }
    if config.set(S::EventsMode).is_some() {
        stores.events = match config.text(S::EventsMode) {
            "warnings" => EventsMode::Warnings,
            "on-demand" => EventsMode::OnDemand,
            _ => EventsMode::All,
        };
    }
    put(&mut stores.max_events, number(S::MaxEvents).map(size));

    put(&mut settings.metrics.period, time(S::MetricsPeriod));
    put(&mut settings.metrics.memory_limit, number(S::MetricsMemory).map(bytes));
    put(&mut settings.logs.ring.lines, number(S::LogLines).map(size));
    put(&mut settings.logs.ring.bytes, number(S::LogMemory).map(bytes));
    put(&mut settings.logs.streams, number(S::LogStreams).map(size));

    let folders = |id| config.set(id).map(|_| config.list(id).iter().map(|path| expand(path, home)).collect());
    put(&mut settings.env.extra_folders, folders(S::PluginFolders));
    put(&mut settings.env.fixed_folders, folders(S::FixedFolders));
    put(&mut settings.env.shell_limit, time(S::ShellLimit));
    settings
}

fn put<T>(field: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *field = value;
    }
}

fn size(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// A window in bytes. The bounds of the table keep each window below 2^31, the limit of HTTP/2.
fn kib(value: u64) -> u32 {
    u32::try_from(value.saturating_mul(1024)).unwrap_or(u32::MAX)
}

fn mib(value: u64) -> u32 {
    kib(value.saturating_mul(1024))
}

/// A limit of memory in bytes, from MiB.
fn bytes(value: u64) -> usize {
    size(value.saturating_mul(1024 * 1024))
}

/// A path of the file, with `~/` as the home folder.
pub(crate) fn expand(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// The period of the look for changes of the kubeconfig files and the settings file.
pub(crate) fn watch_period(config: &Config) -> Duration {
    config.duration(S::KubeconfigWatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::table::{Form, TABLE};
    use crate::config::values::Value;

    /// Each default of the table is the default of the code: a file that sets each setting
    /// to its default changes nothing.
    #[test]
    fn a_file_with_each_default_of_the_table_gives_the_default_settings_of_the_code() {
        let base = CoreSettings::default();
        assert_eq!(format!("{:?}", apply(&Config::default(), &base, None)), format!("{base:?}"));
        let mut config = Config::default();
        for spec in TABLE {
            let value = match spec.form {
                Form::Number(number) => Value::Number(number.default),
                Form::Choice { default, .. } | Form::Text(default) => Value::Text(default.to_owned()),
                Form::Flag(default) => Value::Flag(default),
                Form::List(default) => Value::List(default.iter().map(|item| (*item).to_owned()).collect()),
            };
            config.values.insert(spec.id, value);
        }
        assert_eq!(format!("{:?}", apply(&config, &base, None)), format!("{base:?}"));
        // The core gives the repaint limit to its notifier itself, as it changes at once.
        assert_eq!(config.number(S::RepaintLimit), u64::from(base.repaints_per_second));
    }

    #[test]
    fn a_value_of_the_file_reaches_its_field_and_a_path_with_a_tilde_the_home_folder() {
        let mut config = Config::default();
        for (id, value) in [
            (S::Keepalive, 20),
            (S::PingTimeout, 7),
            (S::ConnectionWindow, 64),
            (S::PageSize, 100),
            (S::WatchSpread, 0),
            (S::MetricsPeriod, 30),
            (S::LogLines, 5000),
            (S::MaxEvents, 300),
            (S::HedgeMin, 300),
        ] {
            config.values.insert(id, Value::Number(value));
        }
        config.values.insert(S::EventsMode, Value::Text("warnings".to_owned()));
        config.values.insert(S::PluginFolders, Value::List(vec!["~/bin".to_owned(), "/opt/bin".to_owned()]));
        let settings = apply(&config, &CoreSettings::default(), Some(Path::new("/Users/tester")));
        let lanes = [&settings.net.watches, &settings.net.requests, &settings.net.logs];
        let ping = (Duration::from_secs(20), Duration::from_secs(7));
        assert!(lanes.iter().all(|lane| (lane.keepalive_interval, lane.keepalive_timeout) == ping));
        let socket = &settings.net.upgrade.socket;
        assert_eq!((socket.ping_after, socket.answer_limit), ping, "a socket pings as a channel");
        assert_eq!((settings.net.watches.connection_window, settings.net.logs.connection_window), (64 << 20, 64 << 20));
        assert_eq!(settings.net.requests.connection_window, 1 << 30, "the reads keep their own window");
        assert_eq!(settings.net.connect.hedge_min, Duration::from_millis(300));
        assert_eq!((settings.stores.watch.page_size, settings.stores.watch.server_timeout), (100, 60..=60));
        assert_eq!((settings.stores.events, settings.stores.max_events), (EventsMode::Warnings, 300));
        assert_eq!((settings.metrics.period, settings.logs.ring.lines), (Duration::from_secs(30), 5000));
        assert_eq!(settings.env.extra_folders, [PathBuf::from("/Users/tester/bin"), PathBuf::from("/opt/bin")]);
    }
}
