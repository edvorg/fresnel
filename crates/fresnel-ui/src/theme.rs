//! Colors, fonts and spacing. Widgets read colors from [`Palette`], not from literals.

use std::collections::HashMap;

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Theme, Visuals};
use fresnel_core::cluster::ClusterId;

/// Row height of lists and tables. One value keeps all views aligned.
pub(crate) const ROW_HEIGHT: f32 = 24.0;
/// Height of toolbars and tab bars.
pub(crate) const BAR_HEIGHT: f32 = 32.0;
/// Standard inner margin of panels.
pub(crate) const PAD: f32 = 8.0;
/// The text size of the status bar and of table headers.
pub(crate) const SMALL: f32 = 11.5;

/// Semantic colors of one theme.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Palette {
    /// Chrome: top bar, tab bar, status bar.
    pub(crate) bg_chrome: Color32,
    /// Content: cards and tables.
    pub(crate) bg_surface: Color32,
    /// Controls, chips and hovered rows.
    pub(crate) bg_raised: Color32,
    /// Text areas: the journal and the text of a plugin.
    pub(crate) bg_sunken: Color32,
    pub(crate) border: Color32,
    pub(crate) text: Color32,
    pub(crate) text_muted: Color32,
    pub(crate) text_faint: Color32,
    pub(crate) accent: Color32,
    /// Background of a selected item.
    pub(crate) accent_bg: Color32,
    pub(crate) ok: Color32,
    pub(crate) warn: Color32,
    pub(crate) error: Color32,
    pub(crate) info: Color32,
    /// The colors that the clusters get. See [`ClusterColors`].
    pub(crate) clusters: [Color32; 8],
}

pub(crate) const DARK: Palette = Palette {
    bg_chrome: Color32::from_rgb(0x15, 0x17, 0x1c),
    bg_surface: Color32::from_rgb(0x1b, 0x1e, 0x24),
    bg_raised: Color32::from_rgb(0x26, 0x2a, 0x33),
    bg_sunken: Color32::from_rgb(0x11, 0x13, 0x17),
    border: Color32::from_rgb(0x2c, 0x31, 0x3b),
    text: Color32::from_rgb(0xd9, 0xdc, 0xe3),
    text_muted: Color32::from_rgb(0x93, 0x9b, 0xa9),
    text_faint: Color32::from_rgb(0x62, 0x6a, 0x78),
    accent: Color32::from_rgb(0x58, 0xa6, 0xff),
    accent_bg: Color32::from_rgb(0x1d, 0x3a, 0x5f),
    ok: Color32::from_rgb(0x3f, 0xb9, 0x50),
    warn: Color32::from_rgb(0xd2, 0x99, 0x22),
    error: Color32::from_rgb(0xf8, 0x51, 0x49),
    info: Color32::from_rgb(0x58, 0xa6, 0xff),
    clusters: [
        Color32::from_rgb(0x58, 0xa6, 0xff),
        Color32::from_rgb(0xbc, 0x8c, 0xff),
        Color32::from_rgb(0x39, 0xc5, 0xbb),
        Color32::from_rgb(0xf0, 0x88, 0x3e),
        Color32::from_rgb(0xe8, 0x6a, 0xa6),
        Color32::from_rgb(0xa5, 0xd6, 0x5b),
        Color32::from_rgb(0xe3, 0xc2, 0x4d),
        Color32::from_rgb(0x8f, 0x9c, 0xf2),
    ],
};

pub(crate) const LIGHT: Palette = Palette {
    bg_chrome: Color32::from_rgb(0xf1, 0xf3, 0xf5),
    bg_surface: Color32::from_rgb(0xff, 0xff, 0xff),
    bg_raised: Color32::from_rgb(0xe6, 0xe9, 0xee),
    bg_sunken: Color32::from_rgb(0xf8, 0xf9, 0xfb),
    border: Color32::from_rgb(0xd3, 0xd8, 0xdf),
    text: Color32::from_rgb(0x1f, 0x23, 0x28),
    text_muted: Color32::from_rgb(0x59, 0x63, 0x6e),
    text_faint: Color32::from_rgb(0x8c, 0x95, 0x9f),
    accent: Color32::from_rgb(0x09, 0x69, 0xda),
    accent_bg: Color32::from_rgb(0xd4, 0xe6, 0xfb),
    ok: Color32::from_rgb(0x1a, 0x7f, 0x37),
    warn: Color32::from_rgb(0x9a, 0x67, 0x00),
    error: Color32::from_rgb(0xcf, 0x22, 0x2e),
    info: Color32::from_rgb(0x09, 0x69, 0xda),
    clusters: [
        Color32::from_rgb(0x09, 0x69, 0xda),
        Color32::from_rgb(0x82, 0x50, 0xdf),
        Color32::from_rgb(0x0a, 0x7f, 0x78),
        Color32::from_rgb(0xbc, 0x4c, 0x00),
        Color32::from_rgb(0xbf, 0x39, 0x89),
        Color32::from_rgb(0x4d, 0x7c, 0x0f),
        Color32::from_rgb(0x8a, 0x6d, 0x00),
        Color32::from_rgb(0x4f, 0x5b, 0xd5),
    ],
};

/// The palette of the theme that `ui` draws with.
pub(crate) fn palette(ui: &egui::Ui) -> &'static Palette {
    if ui.visuals().dark_mode { &DARK } else { &LIGHT }
}

/// The monospace font, for a clock time and for the text of a plugin.
pub(crate) fn mono() -> FontId {
    FontId::monospace(12.0)
}

/// Installs the text sizes and the visuals of the two themes. Call it one time at start.
pub(crate) fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, visuals(&DARK, Visuals::dark()));
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT, Visuals::light()));
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
            (TextStyle::Monospace, mono()),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.spacing.scroll.bar_width = 8.0;
        // The handle uses the quiet background colors, not the text color.
        style.spacing.scroll.foreground_color = false;
    });
}

fn visuals(p: &Palette, mut v: Visuals) -> Visuals {
    let radius = CornerRadius::same(4);
    v.panel_fill = p.bg_chrome;
    v.window_fill = p.bg_surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);
    v.extreme_bg_color = p.bg_sunken;
    v.faint_bg_color = p.bg_raised.gamma_multiply(0.35);
    v.code_bg_color = p.bg_raised;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.accent_bg;
    v.selection.stroke = Stroke::new(1.0, p.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.bg_surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.bg_fill = p.bg_raised;
    w.inactive.weak_bg_fill = p.bg_raised;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.bg_fill = p.border;
    w.hovered.weak_bg_fill = p.border;
    w.hovered.bg_stroke = Stroke::new(1.0, p.text_faint);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.active.bg_fill = p.accent_bg;
    w.active.weak_bg_fill = p.accent_bg;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.open.bg_fill = p.bg_raised;
    w.open.weak_bg_fill = p.bg_raised;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    v
}

/// The color of each cluster. A widget asks with the identity of the cluster, never with
/// a position in a list, so a fifth cluster or a new sequence cannot give a bad index.
#[derive(Debug, Default)]
pub(crate) struct ClusterColors {
    slots: HashMap<ClusterId, usize>,
    /// The colors that the settings give some clusters, in place of the color of the slot.
    chosen: HashMap<ClusterId, Color32>,
}

impl ClusterColors {
    /// Gives each new cluster the next color. A cluster keeps its color while the app runs.
    pub(crate) fn assign<'a>(&mut self, clusters: impl Iterator<Item = &'a ClusterId>) {
        for id in clusters {
            if !self.slots.contains_key(id) {
                self.slots.insert(id.clone(), self.slots.len());
            }
        }
    }

    /// Takes the colors of the settings. A cluster with no color of its own keeps its slot.
    pub(crate) fn choose(&mut self, chosen: impl Iterator<Item = (ClusterId, Color32)>) {
        self.chosen.clear();
        self.chosen.extend(chosen);
    }

    /// The color of a cluster: its color of the settings, else the color of its slot. After
    /// the last color, the first one comes again.
    pub(crate) fn color(&self, id: &ClusterId, palette: &Palette) -> Color32 {
        if let Some(color) = self.chosen.get(id) {
            return *color;
        }
        let slot = self.slots.get(id).copied().unwrap_or_default();
        palette.clusters.get(slot % palette.clusters.len()).copied().unwrap_or(palette.accent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cluster_keeps_its_color_and_more_clusters_than_colors_start_again() {
        let ids: Vec<ClusterId> = (0..10).map(|index| ClusterId::new(format!("cluster-{index}"))).collect();
        let mut colors = ClusterColors::default();
        colors.assign(ids.iter().take(2));
        let second = colors.color(&ids[1], &DARK);
        assert_eq!(second, DARK.clusters[1]);

        // A new sequence of the list, with more clusters.
        colors.assign(ids.iter().rev());
        assert_eq!(colors.color(&ids[1], &DARK), second);
        assert_eq!(colors.color(&ids[0], &DARK), DARK.clusters[0]);
        let all: Vec<Color32> = ids.iter().map(|id| colors.color(id, &DARK)).collect();
        assert_eq!(all.iter().filter(|color| **color == DARK.clusters[0]).count(), 2, "10 clusters, 8 colors");
        assert_eq!(colors.color(&ClusterId::new("not assigned"), &LIGHT), LIGHT.clusters[0]);
        let red = Color32::from_rgb(0xf8, 0x51, 0x49);
        colors.choose(std::iter::once((ids[1].clone(), red)));
        assert_eq!(
            (colors.color(&ids[1], &DARK), colors.color(&ids[1], &LIGHT)),
            (red, red),
            "the color of the settings"
        );
        assert_eq!(colors.color(&ids[0], &DARK), DARK.clusters[0], "the others keep their slots");
    }
}
