//! The port forwards (plan section 12.3): the Forwards tab of the dock, the dialog of the
//! Forward action, the counts of the status bar, and the question before a close.
//!
//! - `tab`: each forward with its state and the buttons of the chosen one.
//! - `dialog`: the dialog of a new forward. `texts`: the texts of a forward.
//! - `quit`: a close of the window while a forward runs asks first (plan section 11.6).

mod dialog;
mod quit;
mod tab;
mod texts;

pub(crate) use dialog::{ForwardAsk, ForwardDialog};
pub(crate) use quit::QuitGuard;
pub(crate) use tab::ForwardsTab;
pub(crate) use texts::summary;
