//! The meaning of a color. A view selects a tone, and the theme gives its color.

use egui::Color32;
use fresnel_core::app::Health;
use fresnel_core::journal::Level;
use fresnel_core::kinds::ObjectHealth;

use crate::theme::Palette;

/// What a text or a mark tells with its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Normal,
    Muted,
    Faint,
    Ok,
    Info,
    Warn,
    Error,
}

impl Tone {
    pub(crate) fn color(self, palette: &Palette) -> Color32 {
        match self {
            Self::Normal => palette.text,
            Self::Muted => palette.text_muted,
            Self::Faint => palette.text_faint,
            Self::Ok => palette.ok,
            Self::Info => palette.info,
            Self::Warn => palette.warn,
            Self::Error => palette.error,
        }
    }

    /// The tone and the text of the one state of a cluster.
    pub(crate) fn of_health(health: Health) -> (Self, &'static str) {
        match health {
            Health::Off => (Self::Faint, "off"),
            Health::Failed => (Self::Error, "failed"),
            Health::Connecting => (Self::Info, "connecting"),
            Health::Live => (Self::Ok, "live"),
            Health::NoConnection => (Self::Warn, "no connection"),
            Health::NoCredential => (Self::Error, "no credential"),
        }
    }

    /// The tone of the health value of an object.
    pub(crate) fn of_object(health: ObjectHealth) -> Self {
        match health {
            ObjectHealth::Healthy => Self::Ok,
            ObjectHealth::Progressing => Self::Info,
            ObjectHealth::Warning => Self::Warn,
            ObjectHealth::Failed => Self::Error,
        }
    }

    /// The tone and the text of a journal level.
    pub(crate) fn of_level(level: Level) -> (Self, &'static str) {
        match level {
            Level::Info => (Self::Info, "info"),
            Level::Warning => (Self::Warn, "warning"),
            Level::Error => (Self::Error, "error"),
        }
    }
}
