//! The legend of a chart: a dot and the name of each line, and a dash for each fixed line,
//! in a row that the caller chooses, as the line of the heading of the chart. The legend of
//! `egui_plot` lies over the plot: it hid the newest samples, and the lines ran through its
//! names. Each entry acts as in that legend: a click hides its line or shows it again, an
//! Alt+click shows that line alone or each line again, and the pointer on an entry draws its
//! line thicker and in front. A hidden line leaves the bounds of the value axis, so a hidden
//! limit gives the room of the axis to the usage.

use std::collections::BTreeSet;

use egui::{Button, Color32, Id, RichText, Sense, Shape, Stroke, Ui};

use super::{Line, Mark};
use crate::theme::Palette;
use crate::widgets::dot;

/// The space between two entries.
const GAP: f32 = 10.0;

/// One entry of the legend. A line and a fixed line have names of their own: a container
/// can have the name "limit".
#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    Line(String),
    Mark(String),
}

/// The lines and the fixed lines that the user hid in one chart, by their names. The
/// memory of egui keeps them from frame to frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Hidden {
    lines: BTreeSet<String>,
    marks: BTreeSet<String>,
}

impl Hidden {
    fn has(&self, entry: &Entry) -> bool {
        match entry {
            Entry::Line(name) => self.lines.contains(name),
            Entry::Mark(name) => self.marks.contains(name),
        }
    }

    /// A click hides a shown entry and shows a hidden one.
    fn toggle(&mut self, entry: Entry) {
        let (set, name) = match entry {
            Entry::Line(name) => (&mut self.lines, name),
            Entry::Mark(name) => (&mut self.marks, name),
        };
        if !set.remove(&name) {
            set.insert(name);
        }
    }

    /// An Alt+click shows the entry alone. When each other entry is hidden already, it shows
    /// each entry again.
    fn focus(&mut self, entry: &Entry, (lines, marks): (&[Line], &[Mark])) {
        let lines = lines.iter().map(|line| Entry::Line(line.name.clone()));
        let others: Vec<Entry> = lines
            .chain(marks.iter().map(|mark| Entry::Mark(mark.name.clone())))
            .filter(|other| other != entry)
            .collect();
        let alone = others.iter().all(|other| self.has(other));
        *self = Self::default();
        if !alone {
            others.into_iter().for_each(|other| self.toggle(other));
        }
    }
}

/// What a chart shows of its lines in one frame: the legend gives it.
#[derive(Clone, Debug)]
pub(crate) struct Shown {
    hidden: Hidden,
    /// The entry under the pointer: its line is thicker and in front.
    hovered: Option<Entry>,
}

impl Shown {
    /// The lines that are not hidden.
    pub(super) fn lines<'a>(&'a self, lines: &'a [Line]) -> impl Iterator<Item = &'a Line> {
        lines.iter().filter(|line| !self.hidden.lines.contains(&line.name))
    }

    /// The fixed lines that are not hidden.
    pub(super) fn marks<'a>(&'a self, marks: &'a [Mark]) -> impl Iterator<Item = &'a Mark> {
        marks.iter().filter(|mark| !self.hidden.marks.contains(&mark.name))
    }

    /// True for the line of the entry under the pointer.
    pub(super) fn hovers_line(&self, line: &Line) -> bool {
        matches!(&self.hovered, Some(Entry::Line(name)) if *name == line.name)
    }

    /// True for the fixed line of the entry under the pointer.
    pub(super) fn hovers_mark(&self, mark: &Mark) -> bool {
        matches!(&self.hovered, Some(Entry::Mark(name)) if *name == mark.name)
    }
}

/// The place of the hidden entries of the chart `id` in the memory of egui.
fn key(id: Id) -> Id {
    id.with("hidden lines")
}

/// Draws the legend of the chart `id`, and gives what the chart shows in this frame.
pub(super) fn show(ui: &mut Ui, palette: &Palette, id: Id, (lines, marks): (&[Line], &[Mark])) -> Shown {
    let mut hidden: Hidden = ui.data(|data| data.get_temp(key(id))).unwrap_or_default();
    let (mut hovered, mut clicked) = (None, None);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let named = lines.iter().map(|line| (Entry::Line(line.name.clone()), line.color, &line.legend, &line.name));
        let fixed = marks.iter().map(|mark| (Entry::Mark(mark.name.clone()), mark.color, &mark.name, &mark.name));
        for (entry, color, text, name) in named.chain(fixed) {
            let shown = !hidden.has(&entry);
            let color = if shown { color } else { palette.text_faint };
            let (text_color, noun) = match entry {
                Entry::Line(_) => {
                    dot(ui, color);
                    (palette.text, "line")
                }
                Entry::Mark(_) => {
                    dash(ui, color);
                    (palette.text_muted, "fixed line")
                }
            };
            let text = RichText::new(text).color(if shown { text_color } else { palette.text_faint });
            let verb = if shown { "Hides" } else { "Shows" };
            let hint = format!("{verb} the {noun} {name}. Alt+click shows it alone.");
            let response = ui.add(Button::new(text).frame(false)).on_hover_text(hint);
            if response.hovered() {
                hovered = Some(entry.clone());
            }
            if response.clicked() {
                clicked = Some((entry, ui.input(|input| input.modifiers.alt)));
            }
            ui.add_space(GAP);
        }
    });
    match clicked {
        Some((entry, true)) => hidden.focus(&entry, (lines, marks)),
        Some((entry, false)) => hidden.toggle(entry),
        None => {}
    }
    ui.data_mut(|data| data.insert_temp(key(id), hidden.clone()));
    Shown { hidden, hovered }
}

/// A short dashed line in the color of a fixed line, as the chart draws it.
fn dash(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 7.0), Sense::hover());
    let ends = [rect.left_center(), rect.right_center()];
    ui.painter().extend(Shape::dashed_line(&ends, Stroke::new(1.5, color), 3.0, 2.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(name: &str) -> Line {
        Line::new(name.to_owned(), Color32::WHITE, [Some([0.0, 1.0])])
    }

    fn mark(name: &str) -> Mark {
        Mark { name: name.to_owned(), value: 2.0, color: Color32::WHITE }
    }

    /// The names that a chart shows of its lines and of its fixed lines.
    fn shown(hidden: &Hidden, (lines, marks): (&[Line], &[Mark])) -> (Vec<String>, Vec<String>) {
        let shown = Shown { hidden: hidden.clone(), hovered: None };
        let lines = shown.lines(lines).map(|line| line.name.clone()).collect();
        (lines, shown.marks(marks).map(|mark| mark.name.clone()).collect())
    }

    #[test]
    fn a_click_hides_an_entry_and_an_alt_click_shows_it_alone_then_each_one_again() {
        let (lines, marks) = ([line("pod"), line("limit")], [mark("request"), mark("limit")]);
        let all = (vec!["pod".to_owned(), "limit".to_owned()], vec!["request".to_owned(), "limit".to_owned()]);
        let mut hidden = Hidden::default();
        assert_eq!(shown(&hidden, (&lines, &marks)), all);
        hidden.toggle(Entry::Mark("limit".to_owned()));
        assert_eq!(
            shown(&hidden, (&lines, &marks)),
            (all.0.clone(), vec!["request".to_owned()]),
            "the fixed line limit hides, and the container limit stays"
        );
        hidden.toggle(Entry::Mark("limit".to_owned()));
        assert_eq!(shown(&hidden, (&lines, &marks)), all, "a second click shows it again");

        let pod = Entry::Line("pod".to_owned());
        hidden.focus(&pod, (&lines, &marks));
        assert_eq!(shown(&hidden, (&lines, &marks)), (vec!["pod".to_owned()], Vec::new()), "the line alone");
        hidden.focus(&pod, (&lines, &marks));
        assert_eq!(shown(&hidden, (&lines, &marks)), all, "each line again");
        hidden.toggle(pod.clone());
        hidden.focus(&pod, (&lines, &marks));
        assert_eq!(shown(&hidden, (&lines, &marks)), (vec!["pod".to_owned()], Vec::new()), "a hidden line alone");
    }
}
