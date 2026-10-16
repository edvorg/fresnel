//! The table of the settings of the fixed groups (plan sections 12.5 and 16): each setting
//! with its group, its key, its label, its default and its bounds.
//!
//! The reader of the file, the apply step and the settings view read this one table, as the
//! keys read the command table. Thus a setting has one name, one default and one range in
//! each place, and a test holds each default equal to the default of the code.

/// A group of the settings file: the table `[name]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Network,
    Plugins,
    Kubeconfig,
    Appearance,
    Table,
    Metrics,
    Logs,
    Terminal,
    Events,
    Safety,
    Diagnostics,
    Notifications,
    Forwards,
}

impl Group {
    /// The groups in the sequence of the settings view.
    pub const ALL: [Self; 13] = [
        Self::Appearance,
        Self::Table,
        Self::Safety,
        Self::Notifications,
        Self::Forwards,
        Self::Kubeconfig,
        Self::Plugins,
        Self::Network,
        Self::Metrics,
        Self::Logs,
        Self::Terminal,
        Self::Events,
        Self::Diagnostics,
    ];

    /// The name of the table in the file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Plugins => "plugins",
            Self::Kubeconfig => "kubeconfig",
            Self::Appearance => "appearance",
            Self::Table => "table",
            Self::Metrics => "metrics",
            Self::Logs => "logs",
            Self::Terminal => "terminal",
            Self::Events => "events",
            Self::Safety => "safety",
            Self::Diagnostics => "diagnostics",
            Self::Notifications => "notifications",
            Self::Forwards => "forwards",
        }
    }

    /// The group of a table name of the file.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|group| group.name() == name)
    }

    /// The title in the settings view.
    pub fn title(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::Plugins => "Credential plugins",
            Self::Kubeconfig => "Kubeconfig files",
            Self::Appearance => "Appearance",
            Self::Table => "Tables",
            Self::Metrics => "Metrics",
            Self::Logs => "Logs",
            Self::Terminal => "Terminal",
            Self::Events => "Events",
            Self::Safety => "Safety",
            Self::Diagnostics => "Diagnostics",
            Self::Notifications => "Notifications",
            Self::Forwards => "Port forwards",
        }
    }

    /// True for a group whose settings act at the next start of the app. The core builds the
    /// connections, the stores, the sampler and the log views of a cluster with them, and a
    /// change under a running connection would need a second path for each of them.
    pub fn at_start(self) -> bool {
        matches!(self, Self::Network | Self::Plugins | Self::Metrics | Self::Logs | Self::Events)
    }
}

/// The identity of one setting of a fixed group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Setting {
    Keepalive,
    PingTimeout,
    LightRequest,
    StreamBudget,
    MaxConnections,
    HedgePercent,
    HedgeMin,
    HedgeMax,
    ResolveLimit,
    WatchTimeout,
    WatchSpread,
    PluginLimit,
    PluginWait,
    StreamWindow,
    ConnectionWindow,
    ReadWindow,
    ReadConnectionWindow,
    PageSize,
    Lists,
    GetLimit,
    WriteLimit,
    ListIdle,
    ListTotal,
    PluginFolders,
    FixedFolders,
    ShellLimit,
    KubeconfigFiles,
    KubeconfigWatch,
    Theme,
    TimeFormat,
    TimeZone,
    RepaintLimit,
    RowHeight,
    MetricsPeriod,
    MetricsMemory,
    LogLines,
    LogMemory,
    LogStreams,
    TerminalFont,
    TerminalScrollback,
    OptionAsMeta,
    TerminalShells,
    DebugImage,
    NodeShellImage,
    NodeShellNamespace,
    EventsMode,
    MaxEvents,
    ReadOnly,
    Confirm,
    JournalEntries,
    NotifyFailed,
    NotifyLost,
    LostAfter,
    NotifyForward,
    StartSaved,
    ForwardWait,
}

/// The unit of a number. The key of the file ends with it, so the file tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Seconds,
    Millis,
    Percent,
    Count,
    Kib,
    Mib,
    Points,
}

impl Unit {
    /// The short text after a number: "15 s".
    pub fn text(self) -> &'static str {
        match self {
            Self::Seconds => "s",
            Self::Millis => "ms",
            Self::Percent => "%",
            Self::Count => "",
            Self::Kib => "KiB",
            Self::Mib => "MiB",
            Self::Points => "pt",
        }
    }
}

/// A whole number with its bounds. A value outside them takes the nearest bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Number {
    pub default: u64,
    pub min: u64,
    pub max: u64,
    pub unit: Unit,
    /// True for a period or a time limit: 0 is refused, not taken as the lowest bound. A
    /// period of 0 would run a loop with no pause, and a limit of 0 fails each request.
    pub period: bool,
}

/// The form of a value and its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Number(Number),
    /// One text of a list.
    Choice {
        options: &'static [&'static str],
        default: &'static str,
    },
    Flag(bool),
    /// A text that the reader checks with its own rule.
    Text(&'static str),
    /// A list of texts, for example of paths.
    List(&'static [&'static str]),
}

/// One entry of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    pub id: Setting,
    pub group: Group,
    /// The key in the table of the group.
    pub key: &'static str,
    /// The words of the settings view.
    pub label: &'static str,
    pub form: Form,
}

const fn num(id: Setting, group: Group, key: &'static str, range: [u64; 3], unit: Unit, label: &'static str) -> Spec {
    let [default, min, max] = range;
    Spec { id, group, key, label, form: Form::Number(Number { default, min, max, unit, period: false }) }
}

/// A number that is a period or a time limit.
const fn time(id: Setting, group: Group, key: &'static str, range: [u64; 3], unit: Unit, label: &'static str) -> Spec {
    let [default, min, max] = range;
    Spec { id, group, key, label, form: Form::Number(Number { default, min, max, unit, period: true }) }
}

const fn other(id: Setting, group: Group, key: &'static str, label: &'static str, form: Form) -> Spec {
    Spec { id, group, key, label, form }
}

const fn choice(options: &'static [&'static str], default: &'static str) -> Form {
    Form::Choice { options, default }
}

use Group::{
    Appearance, Diagnostics, Events, Forwards, Kubeconfig, Logs, Metrics, Network, Notifications, Plugins, Safety,
    Table, Terminal,
};
use Setting as S;
use Unit::{Count, Kib, Mib, Millis, Percent, Points, Seconds};

/// The themes of the app. "system" follows the setting of macOS.
pub const THEMES: &[&str] = &["system", "dark", "light"];
/// The forms of a time of day.
pub const TIME_FORMATS: &[&str] = &["24h", "12h"];
/// The folders after `PATH` where the app looks for a credential plugin (plan section 8.1):
/// none on Windows, where the process has the full `PATH` of the user.
pub const FIXED_FOLDERS: &[&str] = fresnel_net::env::FIXED_FOLDERS;
/// The shells of a terminal, the first that exists in the container (plan section 12.8).
pub const SHELLS: &[&str] = &["/bin/bash", "/bin/sh"];
/// The image of a debug container and of a node shell: busybox has `sh`, `ps` and `nsenter`.
pub const SHELL_IMAGE: &str = "busybox:1.36.1";
/// Which Events a cluster keeps (plan section 9.6).
pub const EVENT_MODES: &[&str] = &["all", "warnings", "on-demand"];
/// The confirm rules: a dialog, or a dialog that waits for the name of the cluster.
pub const CONFIRM_RULES: &[&str] = &["dialog", "type-name"];

/// Each setting of the fixed groups. The defaults are the values that plan section 16 and
/// the measurements of M0 give; `apply` tests hold them equal to the defaults of the code.
pub const TABLE: &[Spec] = &[
    time(S::Keepalive, Network, "keepalive_secs", [10, 1, 60], Seconds, "Ping of a quiet connection after"),
    time(S::PingTimeout, Network, "keepalive_timeout_secs", [5, 1, 60], Seconds, "Close with no answer to a ping in"),
    time(S::LightRequest, Network, "light_request_secs", [30, 5, 85], Seconds, "Light request of an idle channel each"),
    num(S::StreamBudget, Network, "stream_budget_percent", [70, 10, 95], Percent, "Streams of the server limit"),
    num(S::MaxConnections, Network, "max_connections", [3, 1, 8], Count, "Connections of one purpose"),
    num(S::HedgePercent, Network, "hedge_percent", [150, 100, 400], Percent, "Second connect after the connect time ×"),
    time(S::HedgeMin, Network, "hedge_min_ms", [150, 50, 1000], Millis, "Second connect after at least"),
    time(S::HedgeMax, Network, "hedge_max_ms", [1000, 1000, 10_000], Millis, "Second connect after at most"),
    time(S::ResolveLimit, Network, "resolve_limit_secs", [2, 1, 30], Seconds, "Time limit of an address lookup"),
    time(S::WatchTimeout, Network, "watch_timeout_secs", [60, 30, 600], Seconds, "Watch time on the server"),
    num(S::WatchSpread, Network, "watch_timeout_spread_secs", [30, 0, 600], Seconds, "Random time added to it"),
    time(S::PluginLimit, Network, "plugin_limit_secs", [20, 5, 300], Seconds, "Time limit of a plugin run"),
    time(S::PluginWait, Network, "plugin_wait_secs", [2, 1, 30], Seconds, "Wait of a request for a plugin run"),
    num(S::StreamWindow, Network, "stream_window_kib", [1024, 64, 16_384], Kib, "Stream window, watches"),
    num(S::ConnectionWindow, Network, "connection_window_mib", [128, 32, 1024], Mib, "Connection window, watches"),
    num(S::ReadWindow, Network, "read_stream_window_kib", [8192, 64, 16_384], Kib, "Stream window, read"),
    num(S::ReadConnectionWindow, Network, "read_connection_window_mib", [1024, 32, 1024], Mib, "Its connection window"),
    num(S::PageSize, Network, "list_page_size", [500, 50, 5000], Count, "Objects of one list page"),
    num(S::Lists, Network, "lists_at_once", [4, 1, 16], Count, "Lists of a cluster at once"),
    time(S::GetLimit, Network, "get_limit_secs", [10, 2, 120], Seconds, "Time limit of a read"),
    time(S::WriteLimit, Network, "write_limit_secs", [15, 2, 120], Seconds, "Time limit of a write"),
    time(S::ListIdle, Network, "list_idle_secs", [15, 2, 120], Seconds, "List page with no data for"),
    time(S::ListTotal, Network, "list_total_secs", [120, 10, 600], Seconds, "Time limit of a list page"),
    other(S::PluginFolders, Plugins, "folders", "Folders before PATH", Form::List(&[])),
    other(S::FixedFolders, Plugins, "fixed_folders", "Folders after PATH", Form::List(FIXED_FOLDERS)),
    time(S::ShellLimit, Plugins, "shell_limit_secs", [5, 1, 30], Seconds, "Time limit of the login shell"),
    other(S::KubeconfigFiles, Kubeconfig, "files", "Files after ~/.kube/config", Form::List(&[])),
    time(S::KubeconfigWatch, Kubeconfig, "watch_secs", [2, 1, 60], Seconds, "Look for changes each"),
    other(S::Theme, Appearance, "theme", "Theme", choice(THEMES, "system")),
    other(S::TimeFormat, Appearance, "time_format", "Time of day", choice(TIME_FORMATS, "24h")),
    other(S::TimeZone, Appearance, "time_zone", "Time zone", Form::Text("local")),
    num(S::RepaintLimit, Appearance, "repaints_per_second", [60, 10, 240], Count, "Repaints for new data each second"),
    num(S::RowHeight, Table, "row_height", [24, 20, 40], Points, "Height of a row"),
    time(S::MetricsPeriod, Metrics, "period_secs", [15, 10, 120], Seconds, "Poll of metrics-server each"),
    num(S::MetricsMemory, Metrics, "memory_mib", [64, 8, 1024], Mib, "History of one cluster"),
    num(S::LogLines, Logs, "lines", [200_000, 1000, 1_000_000], Count, "Lines of one view"),
    num(S::LogMemory, Logs, "memory_mib", [128, 8, 1024], Mib, "Bytes of one view"),
    num(S::LogStreams, Logs, "streams", [50, 1, 100], Count, "Streams of one view"),
    num(S::TerminalFont, Terminal, "font_size", [13, 8, 32], Points, "Font size"),
    num(S::TerminalScrollback, Terminal, "scrollback_lines", [10_000, 0, 100_000], Count, "Lines of history"),
    other(S::OptionAsMeta, Terminal, "option_as_meta", "Option or Alt key as Meta", Form::Flag(false)),
    other(S::TerminalShells, Terminal, "shells", "Shells to try, in this sequence", Form::List(SHELLS)),
    other(S::DebugImage, Terminal, "debug_image", "Image of a debug container", Form::Text(SHELL_IMAGE)),
    other(S::NodeShellImage, Terminal, "node_shell_image", "Image of a node shell", Form::Text(SHELL_IMAGE)),
    other(
        S::NodeShellNamespace,
        Terminal,
        "node_shell_namespace",
        "Namespace of a node shell",
        Form::Text("kube-system"),
    ),
    other(S::EventsMode, Events, "mode", "Events of a cluster", choice(EVENT_MODES, "all")),
    num(S::MaxEvents, Events, "max", [20_000, 100, 200_000], Count, "Events of a cluster at most"),
    other(S::ReadOnly, Safety, "read_only", "Each cluster is read-only", Form::Flag(false)),
    other(S::Confirm, Safety, "confirm", "Confirm rule", choice(CONFIRM_RULES, "dialog")),
    num(S::JournalEntries, Diagnostics, "journal_entries", [10_000, 1000, 100_000], Count, "Entries of the journal"),
    other(S::NotifyFailed, Notifications, "failed_operations", "A failed operation", Form::Flag(true)),
    other(S::NotifyLost, Notifications, "lost_clusters", "A cluster that stays lost", Form::Flag(true)),
    time(S::LostAfter, Notifications, "lost_after_secs", [60, 10, 3600], Seconds, "Lost while hidden for"),
    other(S::NotifyForward, Notifications, "failed_forwards", "A failed port forward", Form::Flag(true)),
    other(S::StartSaved, Forwards, "start_saved", "Start the saved forwards with the app", Form::Flag(true)),
    time(S::ForwardWait, Forwards, "wait_secs", [5, 1, 60], Seconds, "Wait of a local connection in a break"),
];

/// The entry of a setting. Each setting has one: a test holds it.
pub fn spec(id: Setting) -> Option<&'static Spec> {
    TABLE.iter().find(|spec| spec.id == id)
}

/// The entry with this key in this group.
pub fn find(group: Group, key: &str) -> Option<&'static Spec> {
    TABLE.iter().find(|spec| spec.group == group && spec.key == key)
}

/// The entries of a group, in the sequence of the table.
pub fn of_group(group: Group) -> impl Iterator<Item = &'static Spec> {
    TABLE.iter().filter(move |spec| spec.group == group)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn each_setting_has_one_entry_with_a_unique_key_and_a_default_inside_its_bounds() {
        let ids: BTreeSet<Setting> = TABLE.iter().map(|spec| spec.id).collect();
        assert_eq!(ids.len(), TABLE.len(), "one entry for each setting");
        assert!(ids.iter().all(|id| spec(*id).is_some_and(|spec| spec.id == *id)));
        let keys: BTreeSet<(Group, &str)> = TABLE.iter().map(|spec| (spec.group, spec.key)).collect();
        assert_eq!(keys.len(), TABLE.len(), "a key is unique in its group");
        for spec in TABLE {
            assert_eq!(find(spec.group, spec.key), Some(spec));
            match spec.form {
                Form::Number(number) => {
                    assert!(number.min <= number.default && number.default <= number.max, "{}", spec.key);
                    assert!(!number.period || number.min > 0, "{}: a period has no bound of 0", spec.key);
                }
                Form::Choice { options, default } => assert!(options.contains(&default), "{}", spec.key),
                Form::Flag(_) | Form::Text(_) | Form::List(_) => {}
            }
        }
        assert!(Group::ALL.iter().all(|group| Group::named(group.name()) == Some(*group)));
        assert!(Group::ALL.iter().all(|group| of_group(*group).next().is_some()), "no empty group");
    }

    /// The connection window holds two stream windows at each value of the two bounds, so
    /// the stream budget of plan section 8.7 has room for one stream and the test of a channel.
    #[test]
    fn the_bounds_of_the_windows_keep_room_for_two_streams_and_fit_the_window_of_http2() {
        let bounds = |id| match spec(id).map(|spec| spec.form) {
            Some(Form::Number(number)) => (number.min, number.max),
            other => panic!("{id:?}: {other:?}"),
        };
        for (stream, connection) in
            [(Setting::StreamWindow, Setting::ConnectionWindow), (Setting::ReadWindow, Setting::ReadConnectionWindow)]
        {
            let (largest_stream, smallest_connection) = (bounds(stream).1, bounds(connection).0);
            assert!(smallest_connection * 1024 >= 2 * largest_stream);
            assert!(bounds(connection).1 * 1024 * 1024 < 1 << 31, "HTTP/2 has windows below 2^31");
        }
    }
}
