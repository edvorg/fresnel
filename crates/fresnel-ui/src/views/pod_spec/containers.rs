//! One block for each container of a spec: the init containers, the containers and the
//! ephemeral containers, in the sequence of the spec, as `kubectl describe pod` lists them.

use egui::{Frame, Label, Margin, RichText, Ui};
use fresnel_core::kinds::Kind;
use k8s_openapi::api::core::v1::{
    Container, ContainerPort, ContainerStatus, EnvFromSource, EnvVar, EphemeralContainer, PodSpec, PodStatus, Probe,
    ResourceRequirements, VolumeMount,
};

use super::texts::{self, EnvValue};
use super::{Place, section};
use crate::format::short;
use crate::theme::SMALL;
use crate::views::detail::fields::{ITEM_LIMIT, VALUE_LIMIT, field, more, text_field};
use crate::views::objects::Link;
use crate::widgets::dot;

/// What a block shows of one container. A container and an ephemeral container are two
/// types with the same fields.
struct Shown<'a> {
    name: &'a str,
    role: &'static str,
    image: Option<&'a str>,
    pull: Option<&'a str>,
    command: &'a [String],
    args: &'a [String],
    ports: &'a [ContainerPort],
    env: &'a [EnvVar],
    env_from: &'a [EnvFromSource],
    mounts: &'a [VolumeMount],
    probes: [(&'static str, Option<&'a Probe>); 3],
    resources: Option<&'a ResourceRequirements>,
}

impl<'a> Shown<'a> {
    fn of(container: &'a Container, role: &'static str) -> Self {
        Self {
            name: &container.name,
            role,
            image: container.image.as_deref(),
            pull: container.image_pull_policy.as_deref(),
            command: container.command.as_deref().unwrap_or_default(),
            args: container.args.as_deref().unwrap_or_default(),
            ports: container.ports.as_deref().unwrap_or_default(),
            env: container.env.as_deref().unwrap_or_default(),
            env_from: container.env_from.as_deref().unwrap_or_default(),
            mounts: container.volume_mounts.as_deref().unwrap_or_default(),
            probes: [
                ("Liveness", container.liveness_probe.as_ref()),
                ("Readiness", container.readiness_probe.as_ref()),
                ("Startup", container.startup_probe.as_ref()),
            ],
            resources: container.resources.as_ref(),
        }
    }

    fn of_ephemeral(container: &'a EphemeralContainer) -> Self {
        Self {
            name: &container.name,
            role: "ephemeral container",
            image: container.image.as_deref(),
            pull: container.image_pull_policy.as_deref(),
            command: container.command.as_deref().unwrap_or_default(),
            args: container.args.as_deref().unwrap_or_default(),
            ports: &[],
            env: container.env.as_deref().unwrap_or_default(),
            env_from: container.env_from.as_deref().unwrap_or_default(),
            mounts: container.volume_mounts.as_deref().unwrap_or_default(),
            probes: [("Liveness", None), ("Readiness", None), ("Startup", None)],
            resources: None,
        }
    }
}

/// Draws the section. Gives the object of a link that the user clicked.
pub(super) fn show(ui: &mut Ui, place: &Place<'_>, spec: &PodSpec, status: Option<&PodStatus>) -> Option<Link> {
    let (init, main, ephemeral) = match status {
        Some(status) => (
            statuses(status.init_container_statuses.as_ref()),
            statuses(status.container_statuses.as_ref()),
            statuses(status.ephemeral_container_statuses.as_ref()),
        ),
        None => (&[][..], &[][..], &[][..]),
    };
    section(ui, place.palette, "CONTAINERS");
    let mut clicked = None;
    let init_role = |container: &Container| match container.restart_policy.as_deref() {
        Some("Always") => "sidecar container",
        _ => "init container",
    };
    let blocks = spec
        .init_containers
        .iter()
        .flatten()
        .map(|container| (Shown::of(container, init_role(container)), init))
        .chain(spec.containers.iter().map(|container| (Shown::of(container, "container"), main)))
        .chain(spec.ephemeral_containers.iter().flatten().map(|container| (Shown::of_ephemeral(container), ephemeral)));
    let mut count = 0;
    for (shown, statuses) in blocks {
        count += 1;
        if count > ITEM_LIMIT {
            continue;
        }
        let status = status.is_some().then(|| statuses.iter().find(|status| status.name == shown.name));
        clicked = block(ui, place, &shown, status).or(clicked);
    }
    more(ui, place.palette, count);
    clicked
}

fn statuses(list: Option<&Vec<ContainerStatus>>) -> &[ContainerStatus] {
    list.map(Vec::as_slice).unwrap_or_default()
}

/// One container. `status` is `None` for a template, and `Some(None)` for a container of
/// a pod that has no status yet.
fn block(ui: &mut Ui, place: &Place<'_>, shown: &Shown<'_>, status: Option<Option<&ContainerStatus>>) -> Option<Link> {
    let palette = place.palette;
    let mut clicked = None;
    let frame = Frame::new().fill(palette.bg_raised).corner_radius(4.0).inner_margin(Margin::symmetric(8, 6));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(short(shown.name, VALUE_LIMIT)).strong());
            ui.label(RichText::new(shown.role).size(SMALL).color(palette.text_faint));
        });
        if let Some(status) = status {
            state(ui, place, status);
        }
        text_field(ui, palette, "Image", shown.image.unwrap_or_default());
        optional(ui, place, "Pull policy", shown.pull.map(str::to_owned));
        let ports: Vec<String> = shown.ports.iter().map(texts::port).collect();
        optional(ui, place, "Ports", (!ports.is_empty()).then(|| ports.join(", ")));
        optional(ui, place, "Command", (!shown.command.is_empty()).then(|| shown.command.join(" ")));
        optional(ui, place, "Arguments", (!shown.args.is_empty()).then(|| shown.args.join(" ")));
        clicked = environment(ui, place, shown);
        lines(ui, place, "Mounts", shown.mounts.iter().map(texts::mount));
        for (label, probe) in shown.probes {
            optional(ui, place, label, probe.map(texts::probe));
        }
        let resources = shown.resources;
        optional(ui, place, "Requests", resources.and_then(|r| texts::resources(r.requests.as_ref())));
        optional(ui, place, "Limits", resources.and_then(|r| texts::resources(r.limits.as_ref())));
    });
    ui.add_space(6.0);
    clicked
}

/// The state, the readiness, the restarts and the last state of a container of a pod.
fn state(ui: &mut Ui, place: &Place<'_>, status: Option<&ContainerStatus>) {
    let palette = place.palette;
    let ready = status.is_some_and(|status| status.ready);
    let (text, tone) = texts::state(status.and_then(|status| status.state.as_ref()), ready, place.clock);
    field(ui, palette, "State", |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            dot(ui, tone.color(palette));
            ui.add(Label::new(RichText::new(short(&text, VALUE_LIMIT)).color(tone.color(palette))).wrap());
        });
    });
    let Some(status) = status else {
        return;
    };
    text_field(ui, palette, "Ready", if status.ready { "yes" } else { "no" });
    text_field(ui, palette, "Restarts", &status.restart_count.to_string());
    optional(ui, place, "Last state", texts::last_state(status.last_state.as_ref(), place.clock));
}

/// The environment: each variable with its value, or a link to the ConfigMap or the Secret
/// of its key, and each object whose keys all come in.
fn environment(ui: &mut Ui, place: &Place<'_>, shown: &Shown<'_>) -> Option<Link> {
    let palette = place.palette;
    let mut clicked = None;
    if !shown.env.is_empty() {
        field(ui, palette, "Environment", |ui| {
            for var in shown.env.iter().take(ITEM_LIMIT) {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new(short(&var.name, VALUE_LIMIT)).color(palette.text_muted));
                    ui.label(RichText::new("=").color(palette.text_faint));
                    match texts::env(var) {
                        EnvValue::Text(text) => {
                            ui.add(Label::new(short(text, VALUE_LIMIT)).wrap());
                        }
                        EnvValue::Key { kind, name, key } => {
                            ui.label(format!("key {} of", short(key, VALUE_LIMIT)));
                            let text = format!("{} {name}", kind.info().name);
                            if let Some(link) = place.link(ui, &text, kind, name) {
                                clicked = Some(link);
                            }
                        }
                        EnvValue::Field(text) => {
                            ui.label(RichText::new(text).color(palette.text_muted));
                        }
                    }
                });
            }
            more(ui, palette, shown.env.len());
        });
    }
    if !shown.env_from.is_empty() {
        field(ui, palette, "From", |ui| {
            for source in shown.env_from.iter().take(ITEM_LIMIT) {
                let object = match (&source.config_map_ref, &source.secret_ref) {
                    (Some(map), _) => Some((Kind::ConfigMap, map.name.as_str())),
                    (None, Some(secret)) => Some((Kind::Secret, secret.name.as_str())),
                    (None, None) => None,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label("each key of");
                    if let Some((kind, name)) = object {
                        let text = format!("{} {name}", kind.info().name);
                        if let Some(link) = place.link(ui, &text, kind, name) {
                            clicked = Some(link);
                        }
                    }
                    if let Some(prefix) = source.prefix.as_deref().filter(|prefix| !prefix.is_empty()) {
                        ui.label(RichText::new(format!("with the prefix {prefix}")).color(palette.text_muted));
                    }
                });
            }
            more(ui, palette, shown.env_from.len());
        });
    }
    clicked
}

/// A line of a block for a value that a container can have.
fn optional(ui: &mut Ui, place: &Place<'_>, label: &str, value: Option<String>) {
    if let Some(value) = value {
        text_field(ui, place.palette, label, &value);
    }
}

/// A line with one text for each item of a list.
fn lines(ui: &mut Ui, place: &Place<'_>, label: &str, items: impl ExactSizeIterator<Item = String>) {
    let total = items.len();
    if total == 0 {
        return;
    }
    field(ui, place.palette, label, |ui| {
        for item in items.take(ITEM_LIMIT) {
            ui.add(Label::new(short(&item, VALUE_LIMIT)).wrap());
        }
        more(ui, place.palette, total);
    });
}
