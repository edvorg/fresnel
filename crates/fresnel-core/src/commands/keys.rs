//! The key of each command: the default key of the command table, unless the settings give
//! an other key or none. One key acts for one command only.

use std::collections::BTreeMap;

use super::{CommandId, Shortcut, TABLE};

/// The key of each command that has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    keys: BTreeMap<CommandId, Shortcut>,
}

impl Default for Keys {
    /// The default keys of the table.
    fn default() -> Self {
        Self { keys: TABLE.iter().filter_map(|entry| Some((entry.id, entry.key?))).collect() }
    }
}

impl Keys {
    /// The key of a command. `None` for a command with no key.
    pub fn of(&self, id: CommandId) -> Option<Shortcut> {
        self.keys.get(&id).copied()
    }

    /// Each command with its key, in the sequence of the commands.
    pub fn iter(&self) -> impl Iterator<Item = (CommandId, Shortcut)> + '_ {
        self.keys.iter().map(|(id, key)| (*id, *key))
    }

    /// The default keys with the keys of the settings: `chosen` gives a command its key, or
    /// none, in the sequence of the file. `P` is the place of a choice in the file.
    ///
    /// A key that the user gives to a command is its key, also when it was the default key
    /// of an other command: that command then has no key. When the user gives one key to
    /// two commands, the first one keeps it. Each such case gives a problem at its place.
    pub fn with_choices<P: Copy>(chosen: &[(CommandId, Option<Shortcut>, P)]) -> (Self, Vec<(P, String)>) {
        let mut keys = Self::default();
        for (id, _, _) in chosen {
            keys.keys.remove(id);
        }
        let mut problems = Vec::new();
        for (index, (id, key, place)) in chosen.iter().enumerate() {
            let Some(key) = key else {
                continue;
            };
            let holder = keys.keys.iter().find(|(_, held)| *held == key).map(|(holder, _)| *holder);
            match holder {
                Some(holder)
                    if chosen.get(..index).is_some_and(|before| before.iter().any(|(id, ..)| *id == holder)) =>
                {
                    let text = format!("{key} is the key of `{}` already, so `{}` has no key", name(holder), name(*id));
                    problems.push((*place, text));
                }
                Some(holder) => {
                    keys.keys.remove(&holder);
                    keys.keys.insert(*id, *key);
                    let text = format!("{key} was the default key of `{}`, which now has no key", name(holder));
                    problems.push((*place, text));
                }
                None => {
                    keys.keys.insert(*id, *key);
                }
            }
        }
        (keys, problems)
    }
}

/// The identifier of a command in the settings.
fn name(id: CommandId) -> &'static str {
    super::entry(id).map_or("?", |entry| entry.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::Action;

    fn key(text: &str) -> Shortcut {
        Shortcut::parse(text).expect("a shortcut")
    }

    #[test]
    fn the_settings_change_and_remove_a_key_and_the_others_keep_their_default() {
        let chosen = [(CommandId::Back, Some(key("Alt+Left")), 3), (CommandId::Filter, None, 4)];
        let (keys, problems) = Keys::with_choices(&chosen);
        assert_eq!(problems, []);
        assert_eq!(keys.of(CommandId::Back), Some(key("Alt+Left")));
        assert_eq!(keys.of(CommandId::Filter), None);
        assert_eq!(keys.of(CommandId::QuickSearch), Some(key("Cmd+K")));
        assert_eq!(keys.of(CommandId::Object(Action::Delete)), None, "no default key");
        assert_eq!(Keys::with_choices::<u8>(&[]).0, Keys::default());
    }

    #[test]
    fn one_key_acts_for_one_command_and_each_conflict_is_a_problem_at_its_place() {
        let chosen = [
            // The default key of the quick search goes to Back.
            (CommandId::Back, Some(key("Cmd+K")), 3),
            // A second command with that key gets none.
            (CommandId::Object(Action::Delete), Some(key("Cmd+K")), 4),
            (CommandId::Forward, Some(key("Cmd+Shift+K")), 5),
        ];
        let (keys, problems) = Keys::with_choices(&chosen);
        assert_eq!(keys.of(CommandId::Back), Some(key("Cmd+K")));
        assert_eq!(keys.of(CommandId::QuickSearch), None);
        assert_eq!(keys.of(CommandId::Object(Action::Delete)), None);
        assert_eq!(keys.of(CommandId::Forward), Some(key("Cmd+Shift+K")));
        assert_eq!(keys.of(CommandId::Object(Action::CopyCommand)), None);
        assert_eq!(
            problems,
            [
                (3, "Cmd+K was the default key of `quick-search`, which now has no key".to_owned()),
                (4, "Cmd+K is the key of `back` already, so `delete` has no key".to_owned()),
                (5, "Cmd+Shift+K was the default key of `copy-command`, which now has no key".to_owned()),
            ]
        );
        let mut seen: Vec<Shortcut> = keys.iter().map(|(_, key)| key).collect();
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count, "no key acts for two commands");
    }
}
