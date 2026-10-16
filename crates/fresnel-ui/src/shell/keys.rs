//! The keys of the commands (plan section 11.5): the key of each command of the command
//! table, or the key that the settings give it.

use egui::{Context, KeyboardShortcut, Modifiers};
use fresnel_core::commands::{CommandId, Keys, Shortcut};

/// The shortcut of egui for a shortcut of the settings. `None` for a key that egui does
/// not know: a test holds that each key of the core has its key in egui.
pub(crate) fn of(shortcut: Shortcut) -> Option<KeyboardShortcut> {
    let key = egui::Key::from_name(shortcut.key.name())?;
    let Shortcut { command, ctrl, alt, shift, .. } = shortcut;
    Some(KeyboardShortcut::new(Modifiers { alt, ctrl, shift, mac_cmd: false, command }, key))
}

/// The text of the key of a command in a menu, in the form of the system: `⌘K` on macOS.
pub(crate) fn text(ctx: &Context, keys: &Keys, id: CommandId) -> Option<String> {
    keys.of(id).and_then(of).map(|shortcut| ctx.format_shortcut(&shortcut))
}

/// The command of a key that the user pressed in this frame, if any, with the count of
/// the presses: a slow frame gets each press since the frame before. The input loses that
/// key, so no widget gets it too. While the user types (`typing`), a text field keeps a key
/// that is text, such as `?`.
pub(super) fn pressed(ctx: &Context, keys: &Keys, typing: bool) -> Option<(CommandId, usize)> {
    let mut shortcuts: Vec<(CommandId, KeyboardShortcut)> = keys
        .iter()
        .filter(|(_, shortcut)| !(typing && shortcut.is_text()))
        .filter_map(|(id, shortcut)| Some((id, of(shortcut)?)))
        .collect();
    // A shortcut with fewer modifiers takes a key with more of them too: Cmd+K takes
    // Cmd+Shift+K. So the shortcuts with more modifiers come first.
    shortcuts.sort_by_key(|(_, shortcut)| std::cmp::Reverse(count(shortcut.modifiers)));
    ctx.input_mut(|input| {
        shortcuts.iter().find_map(|(id, shortcut)| {
            let presses = input.count_and_consume_key(shortcut.modifiers, shortcut.logical_key);
            (presses > 0).then_some((*id, presses))
        })
    })
}

/// True when a key of this frame has the Cmd key of macOS: the one key that a terminal leaves
/// to the app. The event tells it, as the state of the modifiers can lag behind it.
pub(super) fn with_cmd(ctx: &Context) -> bool {
    ctx.input(|input| {
        input.modifiers.mac_cmd
            || input.events.iter().any(|event| matches!(event, egui::Event::Key { modifiers, .. } if modifiers.mac_cmd))
    })
}

fn count(modifiers: Modifiers) -> usize {
    [modifiers.alt, modifiers.ctrl, modifiers.shift, modifiers.command].into_iter().filter(|on| *on).count()
}

#[cfg(test)]
mod tests {
    use fresnel_core::commands::Key;

    use super::*;

    #[test]
    fn each_key_of_the_core_is_a_key_of_egui_with_the_same_name() {
        for key in Key::all() {
            let found = egui::Key::from_name(key.name());
            assert!(found.is_some(), "{} is no key of egui", key.name());
        }
        let shortcut = of(Shortcut::parse("Cmd+Shift+[").expect("a shortcut")).expect("a key of egui");
        assert_eq!(shortcut.logical_key, egui::Key::OpenBracket);
        assert_eq!(shortcut.modifiers, Modifiers::COMMAND | Modifiers::SHIFT);
    }

    #[test]
    fn a_key_with_more_modifiers_acts_for_its_own_command() {
        let ctx = Context::default();
        let keys = Keys::default();
        let press = |modifiers: Modifiers| {
            let event =
                egui::Event::Key { key: egui::Key::K, physical_key: None, pressed: true, repeat: false, modifiers };
            let input = egui::RawInput { events: vec![event], ..egui::RawInput::default() };
            let mut command = None;
            ctx.run_ui(input, |ui| command = pressed(ui.ctx(), &keys, false).map(|(id, _)| id))
                .drop_without_applying_deltas();
            command
        };
        let copy_command = CommandId::Object(fresnel_core::kinds::Action::CopyCommand);
        assert_eq!(press(Modifiers::COMMAND | Modifiers::SHIFT), Some(copy_command));
        assert_eq!(press(Modifiers::COMMAND), Some(CommandId::QuickSearch));
        assert_eq!(press(Modifiers::NONE), None, "K with no modifier is text");
    }
}
