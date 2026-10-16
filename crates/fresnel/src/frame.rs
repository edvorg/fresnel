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

/// The URLs that the views asked the system to open in this frame, such as the address of a
/// forward. eframe opens none: its feature for links would start a helper process on Linux.
pub fn take_urls(ctx: &egui::Context) -> Vec<String> {
    ctx.output_mut(|output| {
        let mut urls = Vec::new();
        output.commands.retain(|command| match command {
            egui::OutputCommand::OpenUrl(open) => {
                urls.push(open.url.clone());
                false
            }
            egui::OutputCommand::CopyText(_) | egui::OutputCommand::CopyImage(_) => true,
        });
        urls
    })
}

/// True for a window that is minimized or that lost the focus to some other app. A change
/// of the focus or a minimize gives a frame, so the core learns each change.
fn is_hidden(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        let viewport = input.viewport();
        viewport.minimized == Some(true) || viewport.focused == Some(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Open in browser" of a forward: the window takes the URL out of the frame and opens it
    /// itself; a copy of a text stays for eframe.
    #[test]
    fn the_window_takes_the_urls_of_a_frame_and_leaves_the_other_commands() {
        let ctx = egui::Context::default();
        let mut taken = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.ctx().open_url(egui::OpenUrl::new_tab("http://127.0.0.1:8080/"));
            ui.ctx().copy_text("127.0.0.1:8080".to_owned());
            taken = take_urls(ui.ctx());
        });
        // The test has no painter for the fonts.
        output.textures_delta.clear();
        assert_eq!(taken, ["http://127.0.0.1:8080/"]);
        let copy = egui::OutputCommand::CopyText("127.0.0.1:8080".to_owned());
        assert_eq!(output.platform_output.commands, [copy]);
    }
}
