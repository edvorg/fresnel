//! The run of a command of the command table (plan section 11.5): from its key, from the
//! quick search or from the menu of a view.

use fresnel_core::commands::{Area, CommandId};

use super::sidebar::Page;
use super::{Shell, Tab};

impl Shell {
    /// Runs a command of the command table.
    pub(super) fn run(&mut self, command: CommandId, ctx: &egui::Context) {
        match command {
            CommandId::QuickSearch => self.search.open(),
            CommandId::Back => {
                if let Some(link) = self.history.back() {
                    self.follow_link(link);
                }
            }
            CommandId::Forward => {
                if let Some(link) = self.history.forward() {
                    self.follow_link(link);
                }
            }
            // Cmd+F in an editor finds in its text.
            CommandId::Filter => match self.editors.focused(self.tab) {
                Some(id) => self.editors.find(id),
                None => self.open.focus_filter(),
            },
            CommandId::NewSecret => self.new_secret.open(),
            CommandId::FromYaml => self.tab = Tab::Editor(self.editors.open_create()),
            CommandId::Settings => self.page = Page::Settings,
            CommandId::Forwards => self.tab = Tab::Forwards,
            CommandId::Help => self.help = !self.help,
            CommandId::Focus(area) => self.focus_area(area),
            CommandId::NextArea | CommandId::PreviousArea => {
                let area = self.focus.step(command == CommandId::NextArea);
                self.focus_area(area);
            }
            CommandId::Object(action) => self.open.act(ctx, action),
        }
        ctx.request_repaint();
    }

    /// Puts the focus of the keys into an area. The page of a kind holds the table and the
    /// panel; the sidebar and the dock take theirs in the frame.
    fn focus_area(&mut self, area: Area) {
        self.focus.to(area);
        if matches!(area, Area::Table | Area::Panel) && self.focus.take(area) && matches!(self.page, Page::Kind(_)) {
            self.open.focus(area);
        }
    }
}
