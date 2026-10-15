//! One frame of the window: the views draw a copy of the state of the core, and the core
//! gets the commands of the user (plan section 7.3).
//!
//! The window calls [`draw`] for each frame. A test calls it with no window, so it covers
//! the way from the stores of the core to the rows of a table.

use eframe::egui;
use fresnel_core::app::{Core, CoreView};
use fresnel_ui::Shell;

/// Draws one frame into the root `Ui`. Gives the copy of the state that the frame shows.
pub fn draw(shell: &mut Shell, core: &Core, ui: &mut egui::Ui) -> CoreView {
    core.set_window_hidden(is_hidden(ui.ctx()));
    let view = core.view();
    for command in shell.show(ui, &view) {
        core.send(command);
    }
    view
}

/// True for a window that is minimized or that lost the focus to some other app. A change
/// of the focus or a minimize gives a frame, so the core learns each change.
fn is_hidden(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        let viewport = input.viewport();
        viewport.minimized == Some(true) || viewport.focused == Some(false)
    })
}
