//! The settings (plan section 12.5) on the fake core: the settings view, the read-only
//! clusters and the confirm rule, the themes and the time, the keys and the focus.
//!
//! - `page`: the settings view. `safety`: a read-only cluster and the confirm rule `type-name`.
//! - `appearance`: the settings that act on the window. `keyboard`: the keys and the focus.
//! - `snapshots`: PNG files.

mod appearance;
mod keyboard;
mod page;
mod safety;
mod snapshots;

use std::sync::Arc;

use fresnel_core::app::CoreView;
use fresnel_core::config::{FileRead, FileState, parse};

/// The view of a core that read this text of `config.toml`.
pub(crate) fn with_config(mut view: CoreView, text: &str) -> CoreView {
    let loaded = parse(text);
    assert!(loaded.used && loaded.problems.is_empty(), "{:?}", loaded.problems);
    view.config = Arc::new(loaded.config);
    let read = FileRead::Read { problems: Vec::new(), used: true };
    view.config_file = Arc::new(FileState { read, ..FileState::default() });
    view
}
