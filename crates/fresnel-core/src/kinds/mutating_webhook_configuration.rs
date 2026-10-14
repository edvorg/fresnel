//! The MutatingWebhookConfiguration: its description.

use k8s_openapi::api::admissionregistration::v1::MutatingWebhookConfiguration;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for MutatingWebhookConfiguration {
    const ID: Kind = Kind::MutatingWebhookConfiguration;
    const TITLE: &'static str = "MutatingWebhookConfigurations";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[Column::number("Webhooks", 80.0, |configuration| {
        Value::Number(read::number_of(configuration.webhooks.as_ref().map_or(0, Vec::len)))
    })];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
