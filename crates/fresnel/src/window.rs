//! The window: eframe draws the shell of `fresnel-ui` with the state of the core.
//!
//! No test opens a window, so this module is small. The views have their tests in
//! `fresnel-ui`, and the rest of the program has its tests through [`Boot`].

use std::sync::Arc;

use eframe::egui;
use fresnel_core::app::CoreView;
use fresnel_core::forward::ForwardState;
use fresnel_ui::{Clock, Shell, TerminalFonts};

use crate::boot::Boot;
use crate::nap::NapGuard;
use crate::{frame, notify, platform};

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
        if let Err(error) = platform::quit_closes_the_window() {
            boot.journal().warning("app", format!("Cmd+Q quits with no question: {error}"));
        }
        let shell = Shell::new(&ctx, Clock::system(), &boot.core().tasks());
        let (fonts, problems) = TerminalFonts::of_system();
        problems.into_iter().for_each(|problem| boot.journal().info("app", problem));
        fresnel_ui::add_terminal_fonts(&ctx, fonts);
        boot.core().set_notifier(notify::notifier(boot.journal().clone()));
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
    /// App Nap stays off while a cluster is selected or a forward runs: their channels and
    /// tunnels have timers.
    fn hold_activity(&mut self, view: &CoreView) {
        let forwards = view.forwards.iter().any(|row| row.view.state != ForwardState::Stopped);
        self.nap.hold(forwards || view.clusters.iter().any(|row| row.selected));
    }
}

impl eframe::App for Window {
    /// eframe calls this before each `ui`, and alone while the window is minimized or hidden.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        frame::logic(&mut self.shell, self.boot.core(), ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let view = frame::draw(&mut self.shell, self.boot.core(), ui);
        for url in frame::take_urls(ui.ctx()) {
            if let Err(error) = platform::open_url(&url) {
                self.boot.journal().warning("app", format!("cannot open {url} in a browser: {error}"));
            }
        }
        self.hold_activity(&view);
    }

    /// eframe calls this at the close of the window and at a quit of the system. After such a
    /// quit, macOS ends the process with no return to `main`, so the quit sequence runs here.
    fn on_exit(&mut self) {
        self.nap.hold(false);
        self.boot.quit();
    }
}
