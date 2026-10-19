//! The visual bell of a terminal: the app has no sound for the bell of a remote program, so
//! the tab and the grid of its terminal flash for a short time after each new bell.

use egui::{Context, Response};

/// The time of one flash, in seconds.
const FLASH: f64 = 0.25;
/// The description of a widget during its flash, for a screen reader and the tests.
pub(crate) const RINGS: &str = "bell";

/// The flash of one terminal: the count of bells that it saw, and the end of its flash.
#[derive(Debug, Default)]
pub(crate) struct Bell {
    seen: Option<u64>,
    until: f64,
}

impl Bell {
    /// Takes the count of bells of the terminal now. Gives true while the flash of a new bell
    /// lasts, and asks for the frame at its end. The first look only notes the count: a bell
    /// before the view saw the terminal gives no flash.
    pub(crate) fn flashes(&mut self, ctx: &Context, bells: u64) -> bool {
        let now = ctx.input(|input| input.time);
        if self.seen.is_some_and(|seen| seen != bells) {
            self.until = now + FLASH;
        }
        self.seen = Some(bells);
        let left = self.until - now;
        if left > 0.0 {
            ctx.request_repaint_after_secs(left as f32);
        }
        left > 0.0
    }
}

/// Tells a screen reader and the tests that `response` flashes for a bell.
pub(crate) fn describe(response: &Response) {
    response.ctx.accesskit_node_builder(response.id, |node| node.set_description(RINGS));
}

#[cfg(test)]
mod tests {
    use egui::RawInput;

    use super::*;

    /// A new bell flashes for its time; the count at the first look, and the same count again,
    /// flash not.
    #[test]
    fn a_new_bell_flashes_for_a_short_time() {
        let (ctx, mut bell) = (Context::default(), Bell::default());
        let mut at = |time: f64, bells: u64| {
            let input = RawInput { time: Some(time), ..RawInput::default() };
            let mut flashes = false;
            ctx.run_ui(input, |ui| flashes = bell.flashes(ui.ctx(), bells)).drop_without_applying_deltas();
            flashes
        };
        assert!(!at(1.0, 4), "the bells before the first look");
        assert!(at(1.1, 5) && at(1.3, 5), "a new bell flashes for its time");
        assert!(!at(1.36, 5), "the flash ended");
        assert!(at(2.0, 6));
    }
}
