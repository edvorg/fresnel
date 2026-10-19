//! The port forwards (plan section 12.3): the Forwards tab of the dock, the dialog of the
//! Forward action and the counts of the status bar. The question before a quit is the one of
//! the shell (`shell/quit`).
//!
//! - `tab`: a short line of each forward. `chosen`: the chosen one with its buttons.
//! - `dialog`: the dialog of a new forward. `texts`: the texts of a forward.

mod chosen;
mod dialog;
mod tab;
mod texts;

pub(crate) use dialog::{ForwardAsk, ForwardDialog};
pub(crate) use tab::ForwardsTab;
pub(crate) use texts::summary;
