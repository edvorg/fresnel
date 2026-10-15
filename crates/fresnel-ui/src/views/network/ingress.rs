//! The sections of an Ingress and of an IngressClass (`research/01` section 3.4.3,
//! `research/02` section 4.4). An Ingress has its class, its addresses and its default
//! backend; then its rules with the Service of each path as a link, and its TLS hosts with
//! their Secret. An IngressClass has its controller, its default mark and its parameters.

use egui::Ui;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::networking::v1::{Ingress, IngressBackend, IngressClass};

use crate::views::detail::fields::{NONE, field, heading};
use crate::views::detail::lines::{details_heading, line, none, table};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;

/// The annotation of the class of an Ingress before the field `ingressClassName`.
const OLD_CLASS: &str = "kubernetes.io/ingress.class";
/// The annotation that makes an IngressClass the default one of the cluster.
const DEFAULT_CLASS: &str = "ingressclass.kubernetes.io/is-default-class";

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, ingress: &Ingress) -> Option<Link> {
    let spec = ingress.spec.as_ref();
    let mut clicked = None;
    details_heading(ui, cx);
    let class = spec.and_then(|spec| spec.ingress_class_name.as_deref());
    let old = ingress.metadata.annotations.as_ref().and_then(|annotations| annotations.get(OLD_CLASS));
    field(ui, cx.palette, "Class", |ui| match (class, old) {
        (Some(class), _) => clicked = cx.link(ui, class, Kind::IngressClass, "", class),
        (None, Some(old)) => {
            ui.label(format!("{old} (annotation)"));
        }
        (None, None) => {
            ui.label("the default class of the cluster");
        }
    });
    line(ui, cx, "Addresses", Some(addresses(ingress)));
    if let Some(backend) = spec.and_then(|spec| spec.default_backend.as_ref()) {
        field(ui, cx.palette, "Default backend", |ui| clicked = backend_cell(ui, cx, backend).or(clicked.take()));
    }
    ui.add_space(10.0);
    clicked = rules(ui, cx, ingress).or(clicked);
    heading(ui, cx.palette, "TLS");
    let tls = spec.and_then(|spec| spec.tls.as_deref()).unwrap_or_default();
    if tls.is_empty() {
        none(ui, cx);
    }
    table(ui, cx, "tls", &["Hosts", "Secret"], tls, |ui, entry| {
        let hosts = entry.hosts.as_deref().unwrap_or_default();
        ui.label(if hosts.is_empty() { "each host".to_owned() } else { hosts.join(", ") });
        match entry.secret_name.as_deref() {
            Some(name) => clicked = cx.link(ui, name, Kind::Secret, &cx.id.key.namespace, name).or(clicked.take()),
            None => {
                ui.label(NONE);
            }
        }
    });
    ui.add_space(10.0);
    clicked
}

/// Each path of each rule in one table: the host, the path and its type, and the backend.
fn rules(ui: &mut Ui, cx: &Context<'_>, ingress: &Ingress) -> Option<Link> {
    heading(ui, cx.palette, "RULES");
    let rules = ingress.spec.as_ref().and_then(|spec| spec.rules.as_deref()).unwrap_or_default();
    let mut rows = Vec::new();
    for rule in rules {
        let host = rule.host.as_deref().filter(|host| !host.is_empty()).unwrap_or("*");
        let paths = rule.http.as_ref().map(|http| http.paths.as_slice()).unwrap_or_default();
        rows.extend(paths.iter().map(|path| (host, Some(path))));
        if paths.is_empty() {
            rows.push((host, None));
        }
    }
    if rows.is_empty() {
        none(ui, cx);
    }
    let mut clicked = None;
    table(ui, cx, "rules", &["Host", "Path", "Type", "Backend"], &rows, |ui, (host, path)| {
        ui.label(*host);
        let Some(path) = path else {
            for _ in 0..3 {
                ui.label(NONE);
            }
            return;
        };
        ui.label(path.path.as_deref().unwrap_or("/"));
        ui.label(&path.path_type);
        clicked = backend_cell(ui, cx, &path.backend).or(clicked.take());
    });
    ui.add_space(10.0);
    clicked
}

/// A backend: a Service with its port as a link, or an other object of the namespace.
fn backend_cell(ui: &mut Ui, cx: &Context<'_>, backend: &IngressBackend) -> Option<Link> {
    let namespace = &cx.id.key.namespace;
    if let Some(service) = &backend.service {
        let port = service.port.as_ref();
        let port = port.and_then(|port| port.number.map(|number| number.to_string()).or(port.name.clone()));
        let text = format!("{}:{}", service.name, port.as_deref().unwrap_or(NONE));
        return cx.link(ui, &text, Kind::Service, namespace, &service.name);
    }
    match &backend.resource {
        Some(resource) => cx.reference(ui, &resource.kind, namespace, &resource.name),
        None => {
            ui.label(NONE);
            None
        }
    }
}

/// The addresses of the load balancer of the Ingress, or the note that it has none yet.
fn addresses(ingress: &Ingress) -> String {
    let balancer = ingress.status.as_ref().and_then(|status| status.load_balancer.as_ref());
    let entries = balancer.and_then(|balancer| balancer.ingress.as_deref()).unwrap_or_default();
    let all: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.ip.as_deref().filter(|ip| !ip.is_empty()).or(entry.hostname.as_deref()))
        .collect();
    if all.is_empty() { "none yet: the controller gave no address".to_owned() } else { all.join(", ") }
}

pub(crate) fn class_details(ui: &mut Ui, cx: &Context<'_>, class: &IngressClass) -> Option<Link> {
    let spec = class.spec.as_ref();
    details_heading(ui, cx);
    line(ui, cx, "Controller", spec.and_then(|spec| spec.controller.clone()));
    let default = class.metadata.annotations.as_ref().and_then(|annotations| annotations.get(DEFAULT_CLASS));
    let default =
        if default.map(String::as_str) == Some("true") { "yes: an Ingress with no class gets this one" } else { "no" };
    line(ui, cx, "Default", Some(default.to_owned()));
    let parameters = spec.and_then(|spec| spec.parameters.as_ref()).map(|parameters| {
        let group = parameters.api_group.as_deref().map(|group| format!(" ({group})")).unwrap_or_default();
        let scope = match (parameters.scope.as_deref(), parameters.namespace.as_deref()) {
            (Some("Namespace"), Some(namespace)) => format!(", in the namespace {namespace}"),
            _ => String::new(),
        };
        format!("{} {}{group}{scope}", parameters.kind, parameters.name)
    });
    line(ui, cx, "Parameters", parameters);
    ui.add_space(10.0);
    None
}
