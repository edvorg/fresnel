//! The actions on the object of the panel that this version runs: the Manifest tab, the copy
//! of the name, the copy of a `kubectl` command (its text is in `fresnel_core::ops`) and the
//! keys of a Secret or a ConfigMap. The command table has each other action too; a later
//! milestone gives it its request, and its menu entry is off until then.

use fresnel_core::kinds::Action;

/// True for an action that this version runs.
pub(crate) fn can_run(action: Action) -> bool {
    matches!(
        action,
        Action::ViewManifest
            | Action::CopyName
            | Action::CopyCommand
            | Action::EditKeys
            | Action::AddKey
            | Action::RemoveKey
    )
}

/// Why an action does nothing in this version.
pub(crate) fn later(action: Action) -> String {
    format!("{} comes with a later milestone of the app.", action.label())
}
