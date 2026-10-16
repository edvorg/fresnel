//! The port forwards (plan section 12.3): the Forwards tab of the dock, the dialog of the
//! Forward action, and the counts of the status bar.
//!
//! - `tab`: each forward with its state and the buttons of the chosen one.
//! - `dialog`: the dialog of a new forward. `texts`: the texts of a forward.

mod dialog;
mod tab;
mod texts;

pub(crate) use dialog::{ForwardAsk, ForwardDialog};
pub(crate) use tab::ForwardsTab;
pub(crate) use texts::summary;
