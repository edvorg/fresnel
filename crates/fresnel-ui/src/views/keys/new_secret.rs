//! The dialog of a new Secret (plan section 12.2): its cluster, its namespace, its name, its
//! type and its keys with their values. A type starts with the keys that it needs. The
//! hints of the core tell what the server will refuse; the create goes also with a hint,
//! because the server decides. The dialog shows the state of its create, and a done create
//! gives a button that opens the new Secret.
//!
//! The values live in the dialog only while it is open: a close drops them.

use std::sync::Arc;

use egui::{Align, ComboBox, Context, Id, Key, Layout, Modal, RichText, ScrollArea, TextEdit, Ui};
use fresnel_core::app::Command;
use fresnel_core::cluster::ClusterId;
use fresnel_core::keys::{NewSecret, SERVICE_ACCOUNT_ANNOTATION, SecretType, Value, WriteState};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;

use super::state;
use crate::shell::Scene;
use crate::theme;
use crate::views::objects::Link;
use crate::widgets;

/// The width of the dialog.
const WIDTH: f32 = 600.0;
/// The height of the list of the keys before it scrolls.
const KEYS_HEIGHT: f32 = 320.0;

/// The dialog, open or closed.
pub(crate) struct NewSecretDialog {
    open: bool,
    /// True from the open to the frame that takes the defaults from the view.
    fresh: bool,
    cluster: Option<ClusterId>,
    namespace: String,
    name: String,
    secret_type: SecretType,
    /// Each key with its value, in the sequence of the user.
    rows: Vec<(String, String)>,
    /// The ServiceAccount of a token Secret.
    account: String,
    /// The create that the dialog sent; the view of the core has its state.
    sent: Option<Sent>,
}

/// A create that the dialog sent.
struct Sent {
    secret: NewSecret,
    /// The number of the last create of the view at the click. The job of this create gets
    /// a greater one: the view can still show an earlier create of the same values.
    after: u64,
}

impl Default for NewSecretDialog {
    fn default() -> Self {
        Self {
            open: false,
            fresh: false,
            cluster: None,
            namespace: String::new(),
            name: String::new(),
            secret_type: SecretType::Opaque,
            rows: Vec::new(),
            account: String::new(),
            sent: None,
        }
    }
}

impl NewSecretDialog {
    /// Opens the dialog with no value. The next frame fills the cluster and the namespace.
    pub(crate) fn open(&mut self) {
        *self = Self { open: true, fresh: true, ..Self::default() };
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Draws the dialog while it is open. `namespaces` are the namespaces of the page, for
    /// the menu of the namespace. Gives the new Secret when the user opens it.
    pub(crate) fn show(&mut self, ctx: &Context, scene: &mut Scene<'_>, namespaces: &[Arc<str>]) -> Option<Link> {
        if !self.open {
            return None;
        }
        if std::mem::take(&mut self.fresh) {
            self.defaults(scene);
        }
        let modal = Modal::new(Id::new("new secret")).show(ctx, |ui| {
            ui.set_width(WIDTH);
            self.contents(ui, scene, namespaces)
        });
        let (close, link) = modal.inner;
        if close || ctx.input(|input| input.key_pressed(Key::Escape)) || link.is_some() {
            *self = Self::default();
        }
        link
    }

    /// The first selected cluster with a connection that is not read-only, and the one
    /// namespace of the filter, else the default namespace of the cluster in the settings.
    fn defaults(&mut self, scene: &Scene<'_>) {
        let config = &scene.view.config;
        let connected = scene.view.clusters.iter().filter(|row| row.selected && row.data.is_some());
        let mut writable = connected.clone().filter(|row| !config.read_only(&row.id));
        self.cluster = writable.next().or_else(|| connected.clone().next()).map(|row| row.id.clone());
        let own = self.cluster.as_ref().and_then(|id| config.options(id)).and_then(|o| o.default_namespace.clone());
        let mut filter = scene.namespaces.iter();
        self.namespace = match (filter.next(), filter.next()) {
            (Some(only), None) => only.to_string(),
            _ => own.unwrap_or_else(|| "default".to_owned()),
        };
        self.rows = vec![(String::new(), String::new())];
    }

    /// The form, the hints, the state of the create and the buttons. Gives true for a close.
    fn contents(&mut self, ui: &mut Ui, scene: &mut Scene<'_>, namespaces: &[Arc<str>]) -> (bool, Option<Link>) {
        let palette = theme::palette(ui);
        ui.label(RichText::new("New Secret").heading());
        self.place(ui, scene, namespaces);
        self.type_menu(ui);
        if self.secret_type == SecretType::ServiceAccountToken {
            ui.horizontal(|ui| {
                ui.label("ServiceAccount");
                let field = ui.add(TextEdit::singleline(&mut self.account).desired_width(240.0));
                widgets::name(&field, "ServiceAccount of the token");
            });
        }
        ui.add_space(6.0);
        self.keys(ui);
        let secret = self.secret();
        let twice = self.twice();
        for hint in secret.as_ref().map(NewSecret::hints).unwrap_or_default().iter().chain(&twice) {
            ui.label(RichText::new(hint).color(palette.warn));
        }
        let refusal = self.cluster.as_ref().and_then(|cluster| scene.view.config.refusal(cluster));
        if let Some(refusal) = &refusal {
            ui.label(RichText::new(refusal).color(palette.error));
        }
        let state = self.state(scene);
        let pending = state == Some(WriteState::Pending);
        let created = matches!(state, Some(WriteState::Done(_)));
        if let (Some(state), Some(sent)) = (&state, &self.sent) {
            let (tone, word) = state::word(state);
            let text = match state {
                WriteState::Done(_) => format!("Create: {word}. {} has the Secret now.", sent.secret.cluster),
                WriteState::Pending => format!("Create: {word}…"),
                WriteState::Unknown(error) => {
                    format!("Create: {word}: {error}. The app does not send it again; the list of Secrets shows it.")
                }
                WriteState::Conflict(error) | WriteState::Failed(error) | WriteState::NotSent(error) => {
                    format!("Create: {word}: {error}")
                }
            };
            ui.label(RichText::new(text).color(tone.color(palette)));
        }
        ui.add_space(8.0);
        let mut answer = (false, None);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if created {
                let open = ui.button(RichText::new("Open it").strong());
                if open.clicked() {
                    answer.1 = self.link();
                }
            } else {
                let filled =
                    !self.name.is_empty() && !self.namespace.is_empty() && twice.is_empty() && refusal.is_none();
                let create = ui.add_enabled(
                    secret.is_some() && filled && !pending,
                    egui::Button::new(RichText::new("Create").strong()),
                );
                widgets::name(&create, "Create the Secret");
                if let Some(secret) = secret.filter(|_| create.clicked()) {
                    scene.commands.push(Command::CreateSecret(secret.clone()));
                    let after = scene.view.create.as_ref().map_or(0, |job| job.number);
                    self.sent = Some(Sent { secret, after });
                }
            }
            answer.0 = ui.button("Close").clicked();
        });
        answer
    }

    /// The cluster, the namespace and the name.
    fn place(&mut self, ui: &mut Ui, scene: &Scene<'_>, namespaces: &[Arc<str>]) {
        let palette = theme::palette(ui);
        ui.horizontal(|ui| {
            ui.label("Cluster");
            let shown = self.cluster.as_ref().map_or("none connected", ClusterId::as_str).to_owned();
            let combo = ComboBox::from_id_salt("cluster of the new secret").selected_text(shown);
            let menu = combo.show_ui(ui, |ui| {
                for row in scene.view.clusters.iter().filter(|row| row.selected && row.data.is_some()) {
                    let option = ui.selectable_value(&mut self.cluster, Some(row.id.clone()), row.id.as_str());
                    widgets::name(&option, &format!("Create in {}", row.id));
                }
            });
            widgets::name(&menu.response, "Cluster of the new Secret");
            if let Some(cluster) = &self.cluster {
                widgets::dot(ui, scene.colors.color(cluster, palette));
            }
            ui.label("Namespace");
            let field = ui.add(TextEdit::singleline(&mut self.namespace).desired_width(160.0));
            widgets::name(&field, "Namespace of the new Secret");
            if !namespaces.is_empty() {
                ui.menu_button("Choose", |ui| {
                    for namespace in namespaces {
                        if ui.button(&**namespace).clicked() {
                            self.namespace = namespace.to_string();
                            ui.close();
                        }
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            ui.label("Name");
            let field = ui.add(TextEdit::singleline(&mut self.name).desired_width(240.0));
            widgets::name(&field, "Name of the new Secret");
        });
    }

    /// The type. A new type gives the rows of its keys; a value that the user typed stays
    /// with its key.
    fn type_menu(&mut self, ui: &mut Ui) {
        let before = self.secret_type.clone();
        ui.horizontal(|ui| {
            ui.label("Type");
            let combo = ComboBox::from_id_salt("type of the new secret").selected_text(before.name()).width(280.0);
            let menu = combo.show_ui(ui, |ui| {
                for kind in SecretType::KNOWN {
                    let name = kind.name().to_owned();
                    let option = ui.selectable_value(&mut self.secret_type, kind, name.as_str());
                    widgets::name(&option, &format!("Type {name}"));
                }
            });
            widgets::name(&menu.response, "Type of the new Secret");
        });
        if self.secret_type != before {
            let typed = std::mem::take(&mut self.rows);
            let value = |key: &str| typed.iter().find(|(name, _)| name == key).map(|(_, value)| value.clone());
            let mut rows: Vec<(String, String)> =
                self.secret_type.keys().iter().map(|key| ((*key).to_owned(), value(key).unwrap_or_default())).collect();
            let kept = |(name, value): &(String, String)| !value.is_empty() && !rows.iter().any(|(key, _)| key == name);
            let extra: Vec<(String, String)> = typed.iter().filter(|row| kept(row)).cloned().collect();
            rows.extend(extra);
            if rows.is_empty() {
                rows.push((String::new(), String::new()));
            }
            self.rows = rows;
        }
    }

    /// The rows of the keys, each with a Remove button, and Add key.
    fn keys(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("KEYS").size(11.0).color(theme::palette(ui).text_faint));
        let mut removed = None;
        ScrollArea::vertical().max_height(KEYS_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
            for (index, (key, value)) in self.rows.iter_mut().enumerate() {
                ui.push_id(("new key", index), |ui| {
                    ui.horizontal(|ui| {
                        let name = ui.add(TextEdit::singleline(key).hint_text("Key").desired_width(260.0));
                        widgets::name(&name, &format!("Key {}", index + 1));
                        if ui.small_button("Remove").clicked() {
                            removed = Some(index);
                        }
                    });
                    let field = TextEdit::multiline(value).hint_text("Value").code_editor().desired_rows(2);
                    let field = ui.add(field.desired_width(f32::INFINITY));
                    widgets::name(&field, &format!("Value {}", index + 1));
                });
            }
        });
        if let Some(index) = removed.filter(|index| *index < self.rows.len()) {
            self.rows.remove(index);
        }
        if ui.button("Add key").clicked() {
            self.rows.push((String::new(), String::new()));
        }
    }

    /// The Secret of the form. A row with no name and no value is not a key. `None` with
    /// no cluster.
    fn secret(&self) -> Option<NewSecret> {
        let cluster = self.cluster.clone()?;
        let mut secret = NewSecret::new(cluster, self.namespace.trim(), self.name.trim(), self.secret_type.clone());
        let rows = self.rows.iter().filter(|(key, value)| !key.is_empty() || !value.is_empty());
        secret.entries = rows.map(|(key, value)| (key.clone(), Value::new(value.as_bytes().to_vec()))).collect();
        if self.secret_type == SecretType::ServiceAccountToken {
            secret.annotations.insert(SERVICE_ACCOUNT_ANNOTATION.to_owned(), self.account.trim().to_owned());
        }
        Some(secret)
    }

    /// The state of the create that the dialog sent, `None` before a create. From the click
    /// to the first view of its job, the core did not take it yet: it is pending then, so a
    /// second click does not send it again and get the refusal "already exists".
    fn state(&self, scene: &Scene<'_>) -> Option<WriteState> {
        let sent = self.sent.as_ref()?;
        let job = scene.view.create.as_ref().filter(|job| job.number > sent.after && job.secret == sent.secret);
        Some(job.map_or(WriteState::Pending, |job| job.state.clone()))
    }

    /// A hint for each name of a key that two rows have. The Secret would keep one value
    /// and drop the other one with no word of the server, so the create waits for a change.
    fn twice(&self) -> Vec<String> {
        let mut names: Vec<&str> =
            self.rows.iter().map(|(key, _)| key.as_str()).filter(|key| !key.is_empty()).collect();
        names.sort_unstable();
        let mut twice: Vec<&str> = names
            .windows(2)
            .filter_map(|pair| match pair {
                [first, second] if first == second => Some(*first),
                _ => None,
            })
            .collect();
        twice.dedup();
        twice.into_iter().map(|key| format!("`{key}` is the name of two keys")).collect()
    }

    /// The link of the Secret that the dialog created.
    fn link(&self) -> Option<Link> {
        let sent = &self.sent.as_ref()?.secret;
        Some(Link {
            cluster: sent.cluster.clone(),
            kind: Kind::Secret,
            key: ObjectKey::new(&sent.namespace, &sent.name),
        })
    }
}
