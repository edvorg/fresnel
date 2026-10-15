//! The notifications of the system (plan section 11.6): the core gives a notice for a failed
//! operation and for a cluster that stays lost while the window is hidden, and this module
//! shows it, in the Notification Center of macOS (`platform`). The journal has each notice
//! on each system.

use std::sync::Arc;

use fresnel_core::app::{Notice, Notifier};
use fresnel_core::journal::Journal;

use crate::platform::Notifications;

/// The function that shows a notice, after the question for the permission of the user.
/// `journal` takes each refusal of the system.
pub fn notifier(journal: Journal) -> Notifier {
    let system = Notifications::start(journal);
    Arc::new(move |notice: Notice| system.show(&notice))
}
