//! The dialog of a new forward (plan section 12.3): the port of the object, the local port,
//! fixed or automatic, and the local addresses, by default `127.0.0.1` and `::1`. After the
//! start it shows the state and the address of the forward, with "Copy address" and "Open in
//! browser". A forward reads, so a read-only cluster forwards too.

use egui::{Button, Context, Id, Key, Modal, OpenUrl, RichText, TextEdit, Ui};
use fresnel_core::app::Command;
use fresnel_core::config::forwards::save_edits;
use fresnel_core::forward::{Bind, ForwardRule, LocalPort};
use fresnel_core::ops::Target;

use super::texts;
use crate::shell::Scene;
use crate::theme;
use crate::widgets;

/// The width of the dialog.
const WIDTH: f32 = 520.0;
/// The lowest local port that a program may bind with no right of the system on Linux: a
/// smaller port of the object gets an automatic local port.
const FIRST_FREE_PORT: u16 = 1024;

/// What the Forward action asks for: an object, its ports, and the port of a button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ForwardAsk {
    pub(crate) target: Target,
    /// The ports of the containers of the object, or of the Service.
    pub(crate) ports: Vec<u16>,
    /// The port of the button that the user clicked, if any.
    pub(crate) port: Option<u16>,
}

/// The dialog, open or closed.
#[derive(Debug, Default)]
pub(crate) struct ForwardDialog {
    form: Option<Form>,
}

#[derive(Debug)]
struct Form {
    ask: ForwardAsk,
    port: String,
    automatic: bool,
    local: String,
    everywhere: bool,
    save: bool,
    /// The rule of the start: the dialog then shows its forward.
    started: Option<ForwardRule>,
}

impl ForwardDialog {
    /// Opens the dialog for an object. The port of the button, or else the first port of the
    /// object, is the port; the same number is the local port from 1024 on.
    pub(crate) fn open(&mut self, ask: ForwardAsk) {
        let port = ask.port.or_else(|| ask.ports.first().copied());
        let automatic = port.is_none_or(|port| port < FIRST_FREE_PORT);
        let text = port.map(|port| port.to_string()).unwrap_or_default();
        let local = if automatic { String::new() } else { text.clone() };
        let form = Form { ask, port: text, automatic, local, everywhere: false, save: false, started: None };
        self.form = Some(form);
    }

    pub(crate) fn is_open(&self) -> bool {
        self.form.is_some()
    }

    /// Draws the dialog while it is open.
    pub(crate) fn show(&mut self, ctx: &Context, scene: &mut Scene<'_>) {
        let Some(form) = &mut self.form else { return };
        let modal = Modal::new(Id::new("forward a port")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            form.contents(ui, scene)
        });
        if modal.inner || ctx.input(|input| input.key_pressed(Key::Escape)) {
            self.form = None;
        }
    }
}

impl Form {
    /// The form, or the forward after its start, and the buttons. Gives true for a close.
    fn contents(&mut self, ui: &mut Ui, scene: &mut Scene<'_>) -> bool {
        let palette = theme::palette(ui);
        ui.label(RichText::new("Forward a port").heading());
        let Target { cluster, kind, key, .. } = &self.ask.target;
        let object = format!("{} {}/{} in {cluster}", kind.info().name, key.namespace, key.name);
        ui.label(RichText::new(object).color(palette.text_muted));
        ui.add_space(6.0);
        if let Some(rule) = &self.started {
            return started(ui, scene, rule);
        }
        self.fields(ui);
        let rule = self.rule();
        match &rule {
            Ok(_) if !self.automatic => {
                let text = "A fixed port that an other program holds fails the forward: it never takes an other one.";
                ui.label(RichText::new(text).color(palette.text_faint));
            }
            Ok(_) => {}
            Err(problem) => {
                ui.label(RichText::new(problem).color(palette.warn));
            }
        }
        ui.add_space(8.0);
        let mut close = false;
        ui.horizontal(|ui| {
            let start = ui.add_enabled(rule.is_ok(), Button::new("Start the forward"));
            if start.clicked()
                && let Ok(rule) = rule
            {
                scene.commands.push(Command::StartForward(rule.clone()));
                if self.save {
                    scene.commands.push(Command::EditSettings(save_edits(&rule)));
                }
                self.started = Some(rule);
            }
            close = ui.button("Cancel").clicked();
        });
        close
    }

    /// The port of the object, the local port and the local addresses.
    fn fields(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Port");
            for port in &self.ask.ports {
                let text = port.to_string();
                if ui.selectable_label(self.port == text, &text).clicked() {
                    self.port = text;
                }
            }
            let field = ui.add(TextEdit::singleline(&mut self.port).desired_width(70.0));
            widgets::name(&field, "Port of the object");
        });
        ui.horizontal(|ui| {
            ui.label("Local port");
            ui.radio_value(&mut self.automatic, true, "Automatic");
            ui.radio_value(&mut self.automatic, false, "Fixed");
            let field = ui.add_enabled(!self.automatic, TextEdit::singleline(&mut self.local).desired_width(70.0));
            widgets::name(&field, "Fixed local port");
        });
        ui.horizontal(|ui| {
            ui.label("Address");
            ui.radio_value(&mut self.everywhere, false, "127.0.0.1 and ::1: this computer only");
            ui.radio_value(&mut self.everywhere, true, "Each address: other computers too");
        });
        ui.checkbox(&mut self.save, "Save the forward in the settings: it starts with the app");
    }

    /// The rule of the form, or why it has none.
    fn rule(&self) -> Result<ForwardRule, String> {
        let port = |text: &str, what: &str| match text.trim().parse::<u16>() {
            Ok(port) if port > 0 => Ok(port),
            _ => Err(format!("The {what} is a number from 1 to 65535.")),
        };
        let local = match self.automatic {
            true => LocalPort::Automatic,
            false => LocalPort::Fixed(port(&self.local, "local port")?),
        };
        let Target { cluster, kind, key, .. } = &self.ask.target;
        Ok(ForwardRule {
            cluster: cluster.clone(),
            kind: *kind,
            key: key.clone(),
            port: port(&self.port, "port of the object")?,
            local,
            bind: if self.everywhere { Bind::Everywhere } else { Bind::Localhost },
        })
    }
}

/// The forward of the start, as the core shows it, with its address. Gives true for a close.
fn started(ui: &mut Ui, scene: &Scene<'_>, rule: &ForwardRule) -> bool {
    let palette = theme::palette(ui);
    let row = scene.view.forwards.iter().find(|row| &row.view.rule == rule);
    let (state, tone) = match row {
        Some(row) => texts::state(&row.view, scene.clock),
        None => ("starting".to_owned(), crate::widgets::tone::Tone::Info),
    };
    ui.horizontal(|ui| {
        widgets::dot(ui, tone.color(palette));
        ui.label(RichText::new(state).color(tone.color(palette)));
    });
    let address = row.and_then(|row| row.view.address());
    let text = address.map_or_else(|| "The local port binds.".to_owned(), |address| format!("Address {address}"));
    ui.label(RichText::new(text).font(theme::mono()));
    ui.add_space(8.0);
    let mut close = false;
    ui.horizontal(|ui| {
        if ui.add_enabled(address.is_some(), Button::new("Copy address")).clicked()
            && let Some(address) = address
        {
            ui.ctx().copy_text(address.to_string());
        }
        if ui.add_enabled(address.is_some(), Button::new("Open in browser")).clicked()
            && let Some(address) = address
        {
            ui.ctx().open_url(OpenUrl::new_tab(format!("http://{address}/")));
        }
        close = ui.button("Close").clicked();
    });
    close
}
