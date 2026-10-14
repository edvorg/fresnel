//! The columns of the app that show what a column of `kubectl get` shows, and their
//! comparison with the table that the server prints for kubectl (`as=Table`).
//!
//! The cluster test `kinds` compares them on k3s and writes a copy of each table to
//! `fixtures/kinds`, and a unit test compares them on that copy. The module that includes
//! this file names the module `kinds` of `fresnel-core` as `kinds`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::kinds::{Describe, Kind, Value, Visit, columns};

/// For each kind: a column of the app and the column of the server with the same text.
/// A name of the server that starts with `#` is a count of the items of the column of the
/// app, and one that starts with `^` has the text of the app as its first word.
pub(crate) const SAME: &[(Kind, &[(&str, &str)])] = &[
    (
        Kind::Node,
        &[
            ("Status", "Status"),
            ("Roles", "Roles"),
            ("Version", "Version"),
            ("Internal IP", "Internal-IP"),
            ("External IP", "External-IP"),
            ("OS image", "OS-Image"),
            ("Kernel", "Kernel-Version"),
            ("Container runtime", "Container-Runtime"),
        ],
    ),
    (Kind::Namespace, &[("Status", "Status")]),
    (
        Kind::Pod,
        &[
            ("Ready", "Ready"),
            ("Status", "Status"),
            ("Restarts", "^Restarts"),
            ("IP", "IP"),
            ("Node", "Node"),
            ("Nominated node", "Nominated Node"),
            ("Readiness gates", "Readiness Gates"),
        ],
    ),
    (
        Kind::Deployment,
        &[
            ("Ready", "Ready"),
            ("Updated", "Up-to-date"),
            ("Available", "Available"),
            ("Containers", "Containers"),
            ("Images", "Images"),
            ("Selector", "Selector"),
        ],
    ),
    (Kind::StatefulSet, &[("Ready", "Ready"), ("Containers", "Containers"), ("Images", "Images")]),
    (
        Kind::DaemonSet,
        &[
            ("Desired", "Desired"),
            ("Current", "Current"),
            ("Ready", "Ready"),
            ("Updated", "Up-to-date"),
            ("Available", "Available"),
            ("Node selector", "Node Selector"),
            ("Containers", "Containers"),
            ("Images", "Images"),
            ("Selector", "Selector"),
        ],
    ),
    (
        Kind::ReplicaSet,
        &[
            ("Desired", "Desired"),
            ("Current", "Current"),
            ("Ready", "Ready"),
            ("Containers", "Containers"),
            ("Images", "Images"),
            ("Selector", "Selector"),
        ],
    ),
    (
        Kind::ReplicationController,
        &[
            ("Desired", "Desired"),
            ("Current", "Current"),
            ("Ready", "Ready"),
            ("Containers", "Containers"),
            ("Images", "Images"),
            ("Selector", "Selector"),
        ],
    ),
    (
        Kind::Job,
        &[("Status", "Status"), ("Completions", "Completions"), ("Containers", "Containers"), ("Images", "Images")],
    ),
    (
        Kind::CronJob,
        &[
            ("Schedule", "Schedule"),
            ("Time zone", "Timezone"),
            ("Suspended", "Suspend"),
            ("Active", "Active"),
            ("Containers", "Containers"),
            ("Images", "Images"),
        ],
    ),
    (
        Kind::Service,
        &[
            ("Type", "Type"),
            ("Cluster IP", "Cluster-IP"),
            ("External IP", "External-IP"),
            ("Ports", "Port(s)"),
            ("Selector", "Selector"),
        ],
    ),
    (Kind::EndpointSlice, &[("Address type", "AddressType"), ("Ports", "Ports"), ("Endpoints", "Endpoints")]),
    (Kind::Endpoints, &[("Endpoints", "Endpoints")]),
    (Kind::ConfigMap, &[("Keys", "#Data")]),
    (Kind::Secret, &[("Type", "Type"), ("Keys", "#Data")]),
    (
        Kind::HorizontalPodAutoscaler,
        &[
            ("Reference", "Reference"),
            ("Targets", "Targets"),
            ("Min pods", "MinPods"),
            ("Max pods", "MaxPods"),
            ("Replicas", "Replicas"),
        ],
    ),
    (
        Kind::PodDisruptionBudget,
        &[
            ("Min available", "Min Available"),
            ("Max unavailable", "Max Unavailable"),
            ("Allowed disruptions", "Allowed Disruptions"),
        ],
    ),
    (Kind::ResourceQuota, &[("Request", "Request"), ("Limit", "Limit")]),
    (Kind::LimitRange, &[]),
    (
        Kind::PriorityClass,
        &[("Value", "Value"), ("Global default", "Global-Default"), ("Preemption policy", "PreemptionPolicy")],
    ),
    (Kind::RuntimeClass, &[("Handler", "Handler")]),
    (Kind::Lease, &[("Holder", "Holder")]),
    (Kind::MutatingWebhookConfiguration, &[("Webhooks", "Webhooks")]),
    (Kind::ValidatingWebhookConfiguration, &[("Webhooks", "Webhooks")]),
    (Kind::Ingress, &[("Class", "Class"), ("Hosts", "Hosts"), ("Address", "Address"), ("Ports", "Ports")]),
    (Kind::IngressClass, &[("Controller", "Controller"), ("Parameters", "Parameters")]),
    (Kind::NetworkPolicy, &[("Pod selector", "Pod-Selector")]),
    (Kind::ServiceAccount, &[("Secrets", "Secrets")]),
    (Kind::Role, &[]),
    (Kind::ClusterRole, &[]),
    (
        Kind::RoleBinding,
        &[("Role", "Role"), ("Users", "Users"), ("Groups", "Groups"), ("Service accounts", "ServiceAccounts")],
    ),
    (
        Kind::ClusterRoleBinding,
        &[("Role", "Role"), ("Users", "Users"), ("Groups", "Groups"), ("Service accounts", "ServiceAccounts")],
    ),
    (Kind::ControllerRevision, &[("Revision", "Revision")]),
    (Kind::PodTemplate, &[("Containers", "Containers"), ("Images", "Images"), ("Pod labels", "Pod Labels")]),
    (
        Kind::CertificateSigningRequest,
        &[("Signer", "SignerName"), ("Requestor", "Requestor"), ("Condition", "Condition")],
    ),
    (Kind::ValidatingAdmissionPolicy, &[("Validations", "Validations"), ("Param kind", "ParamKind")]),
    (Kind::ValidatingAdmissionPolicyBinding, &[("Policy", "PolicyName"), ("Param", "ParamRef")]),
    (Kind::ApiService, &[("Service", "Service"), ("Available", "Available")]),
    (
        Kind::FlowSchema,
        &[
            ("Priority level", "PriorityLevel"),
            ("Matching precedence", "MatchingPrecedence"),
            ("Distinguisher", "DistinguisherMethod"),
            ("Missing priority level", "MissingPL"),
        ],
    ),
    (
        Kind::PriorityLevelConfiguration,
        &[
            ("Type", "Type"),
            ("Nominal shares", "NominalConcurrencyShares"),
            ("Queues", "Queues"),
            ("Hand size", "HandSize"),
            ("Queue length limit", "QueueLengthLimit"),
        ],
    ),
    (
        Kind::Event,
        &[
            ("Type", "Type"),
            ("Reason", "Reason"),
            ("Object", "Object"),
            ("Source", "Source"),
            ("Message", "Message"),
            ("Count", "Count"),
        ],
    ),
];

/// The table of the server for the objects of one kind, with each object in full.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Table {
    pub(crate) column_definitions: Vec<Definition>,
    pub(crate) rows: Vec<Row>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Definition {
    pub(crate) name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Row {
    pub(crate) cells: Vec<Json>,
    pub(crate) object: Json,
}

/// The file of the copy of the table of one kind.
pub(crate) fn fixture(kind: Kind) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/kinds");
    folder.join(format!("{}.json", kind.info().plural))
}

/// Compares each cell of the table that a column of the app shows too, and reads each
/// other column of the app for each object. Gives the count of the cells that agree, and
/// a line for each one that does not.
pub(crate) fn compare(kind: Kind, table: &Table) -> (usize, Vec<String>) {
    kind.visit(Compare { kind, table })
}

struct Compare<'a> {
    kind: Kind,
    table: &'a Table,
}

impl Visit for Compare<'_> {
    type Output = (usize, Vec<String>);

    fn visit<K: Describe>(self) -> Self::Output {
        let same = SAME.iter().find(|(kind, _)| *kind == self.kind).map_or(&[][..], |(_, same)| same);
        let ours = columns::<K>();
        let (mut agree, mut differ) = (0, Vec::new());
        for row in &self.table.rows {
            let object: K = match serde_json::from_value(row.object.clone()) {
                Ok(object) => object,
                Err(error) => {
                    differ.push(format!("{:?}: an object does not fit its type: {error}", self.kind));
                    continue;
                }
            };
            let values: Vec<Value<'_>> = ours.iter().map(|column| (column.read)(&object)).collect();
            let name = object.metadata().name.clone().unwrap_or_default();
            for (title, theirs) in [("Name", "Name")].iter().chain(same) {
                let place = |title: &str| ours.iter().position(|column| column.title == title);
                let value = place(title).and_then(|place| values.get(place));
                let cell = self.cell(row, theirs.trim_start_matches(['#', '^']));
                let (Some(value), Some(cell)) = (value, cell) else {
                    differ.push(format!("{:?}: no column {title} of the app or {theirs} of the server", self.kind));
                    continue;
                };
                match same_text(value, cell, theirs) {
                    Some(true) => agree += 1,
                    Some(false) => {
                        differ
                            .push(format!("{:?} {name}: {title} is {value:?}, and the server shows {cell}", self.kind));
                    }
                    None => {}
                }
            }
        }
        (agree, differ)
    }
}

impl Compare<'_> {
    fn cell<'a>(&self, row: &'a Row, name: &str) -> Option<&'a Json> {
        let place = self.table.column_definitions.iter().position(|definition| definition.name == name)?;
        row.cells.get(place)
    }
}

/// True when the value of the app has the text of the cell. `None` for a cell that kubectl
/// cut, which shows only a part of a list.
fn same_text(value: &Value<'_>, cell: &Json, theirs: &str) -> Option<bool> {
    let text = match cell {
        Json::String(text) if text.ends_with(" more...") => return None,
        Json::String(text) => text.clone(),
        Json::Null => String::new(),
        other => other.to_string(),
    };
    if theirs.starts_with('#') {
        let count = match value {
            Value::List(items) => items.len(),
            _ => 0,
        };
        return Some(text == count.to_string());
    }
    let text =
        if theirs.starts_with('^') { text.split_whitespace().next().unwrap_or_default().to_owned() } else { text };
    Some(normal(our_text(value).as_deref()) == normal(Some(&text)))
}

/// The text of a value of the app, as kubectl writes it. A time has no text that does
/// not change, so no column with a time is in the list.
fn our_text(value: &Value<'_>) -> Option<String> {
    match value {
        Value::None => None,
        Value::Text(text) => Some(text.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Ratio(part, whole) => Some(format!("{part}/{whole}")),
        Value::Flag(flag) => Some(flag.to_string()),
        Value::List(items) => Some(items.join(",")),
        Value::Time(_) | Value::Span(_) => Some(format!("{value:?}")),
    }
}

/// The text with the marks of kubectl for no value as none, with no `<` and `>` around a
/// word, in lower case, and with no space after a comma: kubectl joins some lists with
/// `, ` and some with `,`.
fn normal(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    if ["", "<none>", "<unset>", "N/A", "?"].contains(&text) {
        return None;
    }
    let text = text.strip_prefix('<').and_then(|text| text.strip_suffix('>')).unwrap_or(text);
    Some(text.to_lowercase().replace(", ", ","))
}
