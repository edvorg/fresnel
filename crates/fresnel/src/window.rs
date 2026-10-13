//! The window: eframe draws the shell of `fresnel-ui` with the state of the core.
//!
//! No test opens a window, so this module is small. The views have their tests in
//! `fresnel-ui`, and the rest of the program has its tests through [`Boot`].

use std::sync::Arc;

use eframe::egui;
use fresnel_core::app::CoreView;
use fresnel_ui::{Clock, Shell};

use crate::boot::Boot;
use crate::frame;
use crate::nap::NapGuard;

const TITLE: &str = "Fresnel";
const SIZE: [f32; 2] = [1280.0, 800.0];
const SMALLEST_SIZE: [f32; 2] = [640.0, 400.0];

/// Opens the window and runs it to its close. The core starts after the window is open,
/// so no network work can delay the first frame.
pub(crate) fn run(boot: &Arc<Boot>) -> Result<(), String> {
    let viewport = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_inner_size(SIZE)
        .with_min_inner_size(SMALLEST_SIZE)
        // With an empty icon, macOS shows the icon of the bundle.
        .with_icon(Arc::new(egui::IconData::default()));
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let boot = Arc::clone(boot);
    let create = move |creation: &eframe::CreationContext<'_>| {
        let ctx = creation.egui_ctx.clone();
        let shell = Shell::new(&ctx, Clock::system(), &boot.core().tasks());
        boot.core().start(move || ctx.request_repaint());
        Ok(Box::new(Window { shell, boot, nap: NapGuard::default() }) as Box<dyn eframe::App>)
    };
    eframe::run_native(TITLE, options, Box::new(create)).map_err(|error| error.to_string())
}

struct Window {
    shell: Shell,
    boot: Arc<Boot>,
    nap: NapGuard,
}

impl Window {
    /// App Nap stays off while a cluster is selected: its channels have timers.
    fn hold_activity(&mut self, view: &CoreView) {
        self.nap.hold(view.clusters.iter().any(|row| row.selected));
    }
}

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let view = frame::draw(&mut self.shell, self.boot.core(), ui);
        self.hold_activity(&view);
    }

    /// eframe calls this at the close of the window and at Cmd+Q. After Cmd+Q, macOS ends
    /// the process with no return to `main`, so the quit sequence runs here.
    fn on_exit(&mut self) {
        self.nap.hold(false);
        self.boot.quit();
    }
}
