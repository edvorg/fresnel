//! The ServiceAccount: its description.

use k8s_openapi::api::core::v1::ServiceAccount;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for ServiceAccount {
    const ID: Kind = Kind::ServiceAccount;
    const TITLE: &'static str = "ServiceAccounts";
    const SIDEBAR: Sidebar = Sidebar::AccessControl;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[Column::number("Secrets", 64.0, |account| {
        Value::Number(read::number_of(account.secrets.as_ref().map_or(0, Vec::len)))
    })];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
