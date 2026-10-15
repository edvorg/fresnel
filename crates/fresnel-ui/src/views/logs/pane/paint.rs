//! The layout of one row: the styles of the program, the hits of the search and the
//! selection, as the sections of one galley.
//!
//! A row is a part of the text of a line; each place where a style, a hit or the selection
//! starts or ends cuts it into a section. A control character shows as a space, so each
//! character keeps one column, and the column of a click finds its byte.

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Stroke};
use fresnel_core::logs::{Span, Style};

use super::colors;
use crate::theme::Palette;

/// What the layout of a row reads besides its text.
pub(crate) struct Look<'a> {
    pub(crate) font: FontId,
    pub(crate) palette: &'a Palette,
    pub(crate) dark: bool,
}

/// The marks on a row of a line: the bytes of each hit of the search, and of the selection.
#[derive(Default)]
pub(crate) struct Marks<'a> {
    pub(crate) hits: &'a [Range<usize>],
    /// True on the line of the current hit, whose hits have a stronger color.
    pub(crate) current: bool,
    pub(crate) selected: Option<Range<usize>>,
}

/// The galley text of the bytes `row` of `text` with the styles of `spans`. A marker of the
/// app has `marker` true: its text is in the color of the app, with no style of a program.
pub(crate) fn job(
    text: &str,
    row: &Range<usize>,
    spans: &[Span],
    marks: &Marks<'_>,
    marker: bool,
    look: &Look<'_>,
) -> LayoutJob {
    let mut cuts: Vec<usize> = vec![row.start, row.end];
    cuts.extend(spans.iter().map(|span| span.start as usize));
    for range in marks.hits.iter().chain(marks.selected.as_ref()) {
        cuts.extend([range.start, range.end]);
    }
    cuts.retain(|cut| row.contains(cut) || *cut == row.end);
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = LayoutJob::default();
    for pair in cuts.windows(2) {
        let [start, end] = [pair.first().copied().unwrap_or(row.start), pair.get(1).copied().unwrap_or(row.end)];
        let Some(part) = text.get(start..end) else {
            continue;
        };
        let style = if marker { None } else { style_at(spans, start) };
        let shown: String = part.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        job.append(&shown, 0.0, format(style, start, marks, marker, look));
    }
    job
}

/// The style of the program at a byte: the last span that starts at or before it.
fn style_at(spans: &[Span], byte: usize) -> Option<Style> {
    let after = spans.partition_point(|span| span.start as usize <= byte);
    after.checked_sub(1).and_then(|last| spans.get(last)).map(|span| span.style)
}

/// The format of the section that starts at `byte`.
fn format(style: Option<Style>, byte: usize, marks: &Marks<'_>, marker: bool, look: &Look<'_>) -> TextFormat {
    let palette = look.palette;
    let style = style.unwrap_or_default();
    let foreground = if style.bold { colors::bold(style.foreground) } else { style.foreground };
    let mut color = match foreground {
        Some(color) => colors::of(color, look.dark),
        None if marker => palette.info,
        None => palette.text,
    };
    let mut background = style.background.map_or(Color32::TRANSPARENT, |color| colors::of(color, look.dark));
    if style.inverse {
        let behind = if background == Color32::TRANSPARENT { palette.bg_sunken } else { background };
        (color, background) = (behind, color);
    }
    if style.dim {
        color = color.gamma_multiply(0.6);
    }
    let hit = marks.hits.iter().any(|hit| hit.contains(&byte));
    if marks.selected.as_ref().is_some_and(|selected| selected.contains(&byte)) {
        background = palette.accent_bg;
    } else if hit {
        background = if marks.current { palette.warn } else { palette.warn.gamma_multiply(0.35) };
        if marks.current {
            color = palette.bg_sunken;
        }
    }
    let line = |on: bool| if on { Stroke::new(1.0, color) } else { Stroke::NONE };
    TextFormat {
        font_id: look.font.clone(),
        color,
        background,
        italics: style.italic || marker,
        underline: line(style.underline),
        strikethrough: line(style.strike),
        ..TextFormat::default()
    }
}

#[cfg(test)]
mod tests {
    use fresnel_core::logs::Color;

    use super::*;
    use crate::theme::DARK;

    fn look() -> Look<'static> {
        Look { font: FontId::monospace(12.0), palette: &DARK, dark: true }
    }

    fn sections(job: &LayoutJob) -> Vec<(String, Color32, Color32)> {
        let text = |range: &Range<egui::text::ByteIndex>| {
            job.text.get(range.start.0..range.end.0).unwrap_or_default().to_owned()
        };
        job.sections
            .iter()
            .map(|section| (text(&section.byte_range), section.format.color, section.format.background))
            .collect()
    }

    #[test]
    fn a_row_has_a_section_for_each_style_hit_and_selection_inside_it() {
        let red = Style { foreground: Some(Color::Palette(1)), ..Style::default() };
        let spans = [Span { start: 0, style: Style::default() }, Span { start: 4, style: red }];
        let text = "the error\tcode here";
        let hits: Vec<Range<usize>> = std::iter::once(4..9).collect();
        let marks = Marks { hits: &hits, current: false, selected: Some(12..19) };
        let job = job(text, &(2..15), &spans, &marks, false, &look());
        let found = sections(&job);
        let texts: Vec<&str> = found.iter().map(|(text, _, _)| text.as_str()).collect();
        assert_eq!(texts, ["e ", "error", " co", "de "], "the tab shows as a space");
        let red = colors::of(Color::Palette(1), true);
        assert_eq!(
            found.get(1).map(|(_, color, background)| (*color, *background)),
            Some((red, DARK.warn.gamma_multiply(0.35)))
        );
        assert_eq!(found.get(3).map(|(_, _, background)| *background), Some(DARK.accent_bg));
        assert_eq!(
            found.first().map(|(_, color, background)| (*color, *background)),
            Some((DARK.text, Color32::TRANSPARENT))
        );
    }

    #[test]
    fn a_marker_has_the_color_of_the_app_and_bold_inverse_and_dim_change_the_colors() {
        let marker = job("started again", &(0..13), &[], &Marks::default(), true, &look());
        assert_eq!(
            marker.sections.first().map(|section| (section.format.color, section.format.italics)),
            Some((DARK.info, true))
        );
        let style = |change: fn(&mut Style)| {
            let mut style = Style { foreground: Some(Color::Palette(2)), ..Style::default() };
            change(&mut style);
            let spans = [Span { start: 0, style }];
            let job = job("x", &(0..1), &spans, &Marks::default(), false, &look());
            job.sections.first().map(|section| (section.format.color, section.format.background))
        };
        let green = colors::of(Color::Palette(2), true);
        assert_eq!(
            style(|style| style.bold = true),
            Some((colors::of(Color::Palette(10), true), Color32::TRANSPARENT))
        );
        assert_eq!(style(|style| style.inverse = true), Some((DARK.bg_sunken, green)));
        assert_eq!(style(|style| style.dim = true), Some((green.gamma_multiply(0.6), Color32::TRANSPARENT)));
    }
}
