//! The confirm dialog of a terminal that changes the cluster (plan section 12.8): a debug
//! container, which the pod keeps for its life, and a node shell, a privileged pod that the
//! app deletes at the end. A shell and an attach change nothing and open with no dialog.

use egui::{Align, Layout, RichText, Ui};
use fresnel_core::terminal::{Program, TerminalRequest};

use super::dialog::{Close, title, warning};
use super::{Subject, TypedName};
use crate::shell::Scene;
use crate::theme;
use crate::widgets;

/// A terminal that the user confirms before it changes the cluster.
pub(crate) struct TerminalConfirm {
    subject: Subject,
    request: TerminalRequest,
    typed: TypedName,
}

impl TerminalConfirm {
    pub(crate) fn new(subject: Subject, request: TerminalRequest) -> Self {
        Self { subject, request, typed: TypedName::default() }
    }

    /// Draws the dialog. Gives its end.
    pub(super) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>) -> Close {
        let palette = theme::palette(ui);
        title(ui, &self.title());
        ui.label(RichText::new(self.subject.text()).strong());
        ui.add_space(4.0);
        for text in self.warnings() {
            warning(ui, palette, text);
        }
        let cluster = std::iter::once(&self.subject.target.cluster);
        let typed = self.typed.show(ui, palette, scene.view, cluster);
        ui.add_space(8.0);
        if let Some(reason) = &typed {
            ui.add(egui::Label::new(RichText::new(reason).color(palette.error)).wrap());
        }
        let mut close = Close::Open;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let confirm = ui.add_enabled(typed.is_none(), egui::Button::new("Open"));
            widgets::name(&confirm, "Open the terminal");
            if confirm.clicked() {
                close = Close::Terminal(self.request.clone());
            }
            if ui.button("Cancel").clicked() {
                close = Close::Cancelled;
            }
        });
        close
    }

    fn title(&self) -> String {
        match self.request.program {
            Program::Debug => "Add a debug container".to_owned(),
            Program::NodeShell => "Open a shell on the node".to_owned(),
            Program::Shell | Program::Attach => "Open a terminal".to_owned(),
        }
    }

    /// The warnings of the two programs that change the cluster.
    fn warnings(&self) -> &'static [&'static str] {
        match self.request.program {
            Program::Debug => &[
                "A debug container is added to the pod and shares the process namespace of its target. The pod keeps the container for its whole life; it cannot be removed.",
            ],
            Program::NodeShell => &[
                "A privileged pod is created on the node, in the host namespaces, with nsenter into the node. It can change the node. The app deletes the pod when the tab closes.",
            ],
            Program::Shell | Program::Attach => &[],
        }
    }
}
