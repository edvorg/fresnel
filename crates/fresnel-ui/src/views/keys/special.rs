//! The special views of two types of a Secret (plan section 12.2, research 08 section 8.4):
//! each certificate of a TLS Secret with its validity, and each registry of an image pull
//! Secret with its user. The core reads them from the values of the draft, so they follow
//! an edit. No text here holds a byte of `tls.key` or a password.
//!
//! A value of a megabyte can name thousands of registries or names, and each one is a
//! layout in each frame, so the view shows the first ones of each list and their count.

use egui::{RichText, Ui};
use fresnel_core::keys::{Certificate, Registry, Special};

use crate::format::{Clock, short};
use crate::theme::Palette;
use crate::views::detail::fields::{ITEM_LIMIT, NONE, VALUE_LIMIT, field, heading, more, text_field};
use crate::widgets::tone::Tone;

/// A certificate that ends in fewer days shows in the tone of a warning.
const SOON: i64 = 30;

/// Draws the special view of a Secret. A type with none draws nothing.
pub(super) fn show(ui: &mut Ui, palette: &Palette, clock: &Clock, special: &Special) {
    match special {
        Special::None => {}
        Special::Certificates(found) => {
            ui.add_space(10.0);
            heading(ui, palette, "CERTIFICATES");
            for (index, certificate) in found.iter().enumerate() {
                match certificate {
                    Ok(certificate) => one_certificate(ui, palette, clock, index + 1, certificate),
                    Err(reason) => {
                        ui.label(RichText::new(reason).color(palette.error));
                    }
                }
            }
        }
        Special::Registries(found) => {
            ui.add_space(10.0);
            heading(ui, palette, "REGISTRIES");
            match found {
                Ok(registries) if registries.is_empty() => {
                    ui.label(RichText::new("No registry.").color(palette.text_faint));
                }
                Ok(registries) => {
                    registries.iter().take(ITEM_LIMIT).for_each(|registry| one_registry(ui, palette, registry));
                    more(ui, palette, registries.len());
                }
                Err(reason) => {
                    ui.label(RichText::new(reason).color(palette.error));
                }
            }
        }
    }
}

fn one_certificate(ui: &mut Ui, palette: &Palette, clock: &Clock, number: usize, certificate: &Certificate) {
    let authority = if certificate.is_ca { " · certificate authority" } else { "" };
    ui.label(RichText::new(format!("Certificate {number}{authority}")).strong());
    text_field(ui, palette, "Subject", &certificate.subject);
    text_field(ui, palette, "Issuer", &certificate.issuer);
    let (from, to) = (clock.date_time(certificate.not_before), clock.date_time(certificate.not_after));
    text_field(ui, palette, "Valid", &format!("{from} to {to}"));
    let (tone, end) = expiry(certificate.days_left(clock.now()));
    field(ui, palette, "Expires", |ui| {
        ui.label(RichText::new(end).color(tone.color(palette)));
    });
    let first: Vec<&str> = certificate.names.iter().take(ITEM_LIMIT).map(String::as_str).collect();
    let names = if first.is_empty() { NONE.to_owned() } else { first.join(", ") };
    text_field(ui, palette, "Names", &names);
    more(ui, palette, certificate.names.len());
    text_field(ui, palette, "Serial", &certificate.serial);
    text_field(ui, palette, "Key", &certificate.key);
    ui.add_space(6.0);
}

/// The words and the tone of the end of a validity, from the whole days that are left.
fn expiry(days: i64) -> (Tone, String) {
    match days {
        ..0 => (Tone::Error, format!("expired {} ago", day_count(-days))),
        0 => (Tone::Warn, "in less than a day".to_owned()),
        1..SOON => (Tone::Warn, format!("in {}", day_count(days))),
        _ => (Tone::Ok, format!("in {}", day_count(days))),
    }
}

fn day_count(days: i64) -> String {
    crate::format::count(days.unsigned_abs(), "day")
}

fn one_registry(ui: &mut Ui, palette: &Palette, registry: &Registry) {
    ui.label(RichText::new(short(&registry.server, VALUE_LIMIT)).strong());
    text_field(ui, palette, "User", registry.username.as_deref().unwrap_or(NONE));
    if let Some(email) = &registry.email {
        text_field(ui, palette, "E-mail", email);
    }
    let password = if registry.has_password { "set" } else { "none" };
    text_field(ui, palette, "Password", password);
    ui.add_space(6.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_a_validity_has_the_tone_of_its_distance() {
        assert_eq!(expiry(-1), (Tone::Error, "expired 1 day ago".to_owned()));
        assert_eq!(expiry(0), (Tone::Warn, "in less than a day".to_owned()));
        assert_eq!(expiry(29), (Tone::Warn, "in 29 days".to_owned()));
        assert_eq!(expiry(30), (Tone::Ok, "in 30 days".to_owned()));
    }
}
