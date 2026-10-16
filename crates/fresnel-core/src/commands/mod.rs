//! The command table (plan section 11.5): each command of the app with its identifier, its
//! label, its default key, its kinds and its confirm rule.
//!
//! The keys, the row menus and the quick search of the UI read this one table, and the
//! settings name a command by its identifier. Thus a command has one label and one key in
//! each place, and a new action of a kind gets its entry here or a test fails.
//!
//! - `shortcut`: a key with its modifiers. `keys`: the key of each command after the settings.

mod keys;
mod shortcut;

pub use keys::Keys;
pub use shortcut::{Key, Shortcut};

use crate::kinds::{Action, Kind};

/// The identity of one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CommandId {
    /// Opens the quick search.
    QuickSearch,
    /// Shows the object that the user visited before the one of now.
    Back,
    /// Shows the object that the user visited after the one of now, after a step back.
    Forward,
    /// Puts the focus into the text filter of the table.
    Filter,
    /// Opens the dialog of a new Secret, with its cluster and namespace (plan section 12.2).
    NewSecret,
    /// Opens a tab that creates or applies a text of documents (plan section 12.9).
    FromYaml,
    /// Opens the settings view (plan section 12.5).
    Settings,
    /// Shows the keys of each command over the window.
    Help,
    /// Puts the focus into one area of the window: the sidebar, the table, the panel or the
    /// dock. The keys of that area then act there.
    Focus(Area),
    /// Puts the focus into the next area, or the area before.
    NextArea,
    PreviousArea,
    /// An action on the object of the detail panel.
    Object(Action),
}

/// An area of the window that takes the focus of the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Area {
    Sidebar,
    Table,
    Panel,
    Dock,
}

impl Area {
    /// The areas in the sequence of the next area.
    pub const ALL: [Self; 4] = [Self::Sidebar, Self::Table, Self::Panel, Self::Dock];
}

/// When a command asks the user before it acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// The command reads, or opens a view.
    Never,
    /// The command changes a cluster: a dialog lists each target as cluster, namespace,
    /// kind and name first (plan section 11.2).
    Always,
}

/// One entry of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: CommandId,
    /// The identifier: the name of the command in the settings.
    pub name: &'static str,
    /// The words of a menu entry and of a result of the quick search.
    pub label: &'static str,
    /// The key of the command, unless the settings give an other one.
    pub key: Option<Shortcut>,
    pub confirm: Confirm,
}

impl Entry {
    const fn app(id: CommandId, name: &'static str, label: &'static str, key: Shortcut) -> Self {
        Self { id, name, label, key: Some(key), confirm: Confirm::Never }
    }

    const fn object(action: Action, name: &'static str, key: Option<Shortcut>, confirm: Confirm) -> Self {
        Self { id: CommandId::Object(action), name, label: action.label(), key, confirm }
    }

    /// The kinds whose objects the command acts on, in the sequence of the kinds. A
    /// command of the app acts on no object.
    pub fn kinds(&self) -> impl Iterator<Item = Kind> + use<> {
        let action = match self.id {
            CommandId::Object(action) => Some(action),
            CommandId::QuickSearch
            | CommandId::Back
            | CommandId::Forward
            | CommandId::Filter
            | CommandId::NewSecret
            | CommandId::FromYaml
            | CommandId::Settings
            | CommandId::Help
            | CommandId::Focus(_)
            | CommandId::NextArea
            | CommandId::PreviousArea => None,
        };
        Kind::ALL.iter().copied().filter(move |kind| action.is_some_and(|action| kind.actions().any(|a| a == action)))
    }
}

use Confirm::{Always, Never};

/// The commands of the app and the actions on objects, in the sequence of a menu. Each
/// command that the app runs has a default key; an action of a later milestone gets one
/// with it. A change of a cluster uses Option or Shift, so no key of a text field and no
/// key of macOS (Cmd+C, Cmd+Backspace) starts a dialog of a change.
pub const TABLE: &[Entry] = &[
    Entry::app(CommandId::QuickSearch, "quick-search", "Quick search", Shortcut::command("K")),
    Entry::app(CommandId::Back, "back", "Back", Shortcut::command("OpenBracket")),
    Entry::app(CommandId::Forward, "forward", "Forward", Shortcut::command("CloseBracket")),
    Entry::app(CommandId::Filter, "filter", "Filter the table", Shortcut::command("F")),
    Entry::app(CommandId::NewSecret, "new-secret", "New Secret", Shortcut::command("N").shift()),
    Entry::app(CommandId::FromYaml, "from-yaml", "Create or apply YAML", Shortcut::command("I").shift()),
    Entry::app(CommandId::Settings, "settings", "Settings", Shortcut::command("Comma")),
    Entry::app(CommandId::Help, "help", "Keys", Shortcut::plain("Questionmark")),
    Entry::app(CommandId::Focus(Area::Sidebar), "focus-sidebar", "Focus the sidebar", Shortcut::command("1")),
    Entry::app(CommandId::Focus(Area::Table), "focus-table", "Focus the table", Shortcut::command("2")),
    Entry::app(CommandId::Focus(Area::Panel), "focus-panel", "Focus the panel", Shortcut::command("3")),
    Entry::app(CommandId::Focus(Area::Dock), "focus-dock", "Focus the dock", Shortcut::command("4")),
    Entry::app(CommandId::NextArea, "next-area", "Next area", Shortcut::plain("F6")),
    Entry::app(CommandId::PreviousArea, "previous-area", "Area before", Shortcut::plain("F6").shift()),
    Entry::object(Action::ViewManifest, "view-manifest", Some(Shortcut::command("M").shift()), Never),
    Entry::object(Action::CopyName, "copy-name", Some(Shortcut::command("C").shift()), Never),
    Entry::object(Action::CopyCommand, "copy-command", Some(Shortcut::command("K").shift()), Never),
    Entry::object(Action::Download, "download", Some(Shortcut::command("D").shift()), Never),
    Entry::object(Action::EditManifest, "edit-manifest", Some(Shortcut::command("Y").shift()), Always),
    Entry::object(Action::Logs, "logs", Some(Shortcut::command("L").shift()), Never),
    Entry::object(Action::Terminal, "terminal", Some(Shortcut::command("T").shift()), Never),
    Entry::object(Action::Attach, "attach", Some(Shortcut::command("A").shift()), Never),
    Entry::object(Action::DebugContainer, "debug-container", Some(Shortcut::command("G").shift()), Always),
    Entry::object(Action::Forward, "forward-port", None, Never),
    Entry::object(Action::Scale, "scale", Some(Shortcut::command("S").shift()), Always),
    Entry::object(Action::Restart, "restart", Some(Shortcut::command("R").shift()), Always),
    Entry::object(Action::RolloutStatus, "rollout-status", Some(Shortcut::command("U").shift()), Never),
    Entry::object(Action::RevisionHistory, "revision-history", Some(Shortcut::command("H").shift()), Never),
    Entry::object(Action::Rollback, "rollback", Some(Shortcut::command("B").shift()), Always),
    Entry::object(Action::RunAgain, "run-again", Some(Shortcut::command("A").alt()), Always),
    Entry::object(Action::Suspend, "suspend", Some(Shortcut::command("S").alt()), Always),
    Entry::object(Action::Resume, "resume", Some(Shortcut::command("R").alt()), Always),
    Entry::object(Action::Trigger, "trigger", Some(Shortcut::command("T").alt()), Always),
    Entry::object(Action::Cordon, "cordon", Some(Shortcut::command("C").alt()), Always),
    Entry::object(Action::Uncordon, "uncordon", Some(Shortcut::command("U").alt()), Always),
    Entry::object(Action::Drain, "drain", Some(Shortcut::command("D").alt()), Always),
    Entry::object(Action::NodeShell, "node-shell", Some(Shortcut::command("J").shift()), Always),
    Entry::object(Action::EditKeys, "edit-keys", Some(Shortcut::command("E").shift()), Always),
    Entry::object(Action::AddKey, "add-key", Some(Shortcut::command("N").alt()), Always),
    Entry::object(Action::RemoveKey, "remove-key", Some(Shortcut::command("Delete").alt()), Always),
    Entry::object(Action::Evict, "evict", Some(Shortcut::command("E").alt()), Always),
    Entry::object(Action::Delete, "delete", Some(Shortcut::command("Backspace").alt()), Always),
    Entry::object(Action::ForceDelete, "force-delete", Some(Shortcut::command("Backspace").alt().shift()), Always),
];

/// The entry of a command. Each command has one: a test holds it.
pub fn entry(id: CommandId) -> Option<&'static Entry> {
    TABLE.iter().find(|entry| entry.id == id)
}

/// The entry with this identifier, for a key of the settings.
pub fn named(name: &str) -> Option<&'static Entry> {
    TABLE.iter().find(|entry| entry.name == name)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn each_action_of_each_kind_and_each_command_of_the_app_has_one_entry() {
        let mut wanted: BTreeSet<CommandId> =
            Kind::ALL.iter().flat_map(|kind| kind.actions()).map(CommandId::Object).collect();
        let app = [
            CommandId::QuickSearch,
            CommandId::Back,
            CommandId::Forward,
            CommandId::Filter,
            CommandId::NewSecret,
            CommandId::FromYaml,
        ];
        wanted.extend(app);
        wanted.extend([CommandId::Settings, CommandId::Help, CommandId::NextArea, CommandId::PreviousArea]);
        wanted.extend(Area::ALL.map(CommandId::Focus));
        let ids: Vec<CommandId> = TABLE.iter().map(|entry| entry.id).collect();
        assert_eq!(ids.iter().copied().collect::<BTreeSet<_>>(), wanted, "each command, and no other one");
        assert_eq!(ids.len(), wanted.len(), "one entry for each command");
        assert!(wanted.iter().all(|id| entry(*id).is_some_and(|entry| entry.id == *id)));
    }

    #[test]
    fn the_identifiers_the_labels_and_the_default_keys_are_unique() {
        let unique = |values: Vec<String>| values.iter().collect::<BTreeSet<_>>().len() == values.len();
        assert!(unique(TABLE.iter().map(|entry| entry.name.to_owned()).collect()));
        assert!(unique(TABLE.iter().map(|entry| entry.label.to_owned()).collect()));
        assert!(unique(TABLE.iter().filter_map(|entry| entry.key).map(|key| key.to_string()).collect()));
        let well_formed =
            |name: &str| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-');
        assert!(TABLE.iter().all(|entry| well_formed(entry.name)));
        assert_eq!(named("back").map(|entry| entry.id), Some(CommandId::Back));
        assert_eq!(named("Back"), None, "an identifier is in small letters");
    }

    /// The text of each default key reads back as the same key, so the settings can name it.
    #[test]
    fn each_default_key_is_a_key_of_the_list_and_reads_back_from_its_text() {
        for entry in TABLE {
            if let Some(key) = entry.key {
                assert_eq!(Key::named(key.key.name()), Some(key.key), "{}", entry.name);
                assert_eq!(Shortcut::parse(&key.to_string()), Ok(key), "{}", entry.name);
            }
        }
    }

    #[test]
    fn a_command_of_the_app_has_no_kind_and_an_action_has_the_kinds_that_have_it() {
        let kinds = |id| entry(id).map(|entry| entry.kinds().collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!(kinds(CommandId::Back), []);
        assert_eq!(kinds(CommandId::NewSecret), [], "a new Secret is no action on an object");
        assert_eq!(kinds(CommandId::Object(Action::Drain)), [Kind::Node]);
        assert_eq!(kinds(CommandId::Object(Action::Delete)).len(), Kind::ALL.len(), "an action of each kind");
        let changes = [Action::Delete, Action::Drain, Action::Scale, Action::EditKeys];
        assert!(changes.iter().all(|action| entry(CommandId::Object(*action)).is_some_and(|e| e.confirm == Always)));
        let reads = [Action::ViewManifest, Action::CopyName, Action::Logs];
        assert!(reads.iter().all(|action| entry(CommandId::Object(*action)).is_some_and(|e| e.confirm == Never)));
    }
}
