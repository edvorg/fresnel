//! The Lease: its description. Its objects change all the time, so it has a store only
//! while a view needs it (plan section 9.5).

use std::time::SystemTime;

use k8s_openapi::api::coordination::v1::Lease;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for Lease {
    const ID: Kind = Kind::Lease;
    const TITLE: &'static str = "Leases";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::C;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Holder", 220.0, |lease| {
            Value::text(lease.spec.as_ref().and_then(|spec| spec.holder_identity.as_deref()))
        }),
        Column::number("Renewed", 90.0, |lease: &Self| {
            let renewed = lease.spec.as_ref().and_then(|spec| spec.renew_time.as_ref());
            renewed.map_or(Value::None, |time| Value::Time(SystemTime::from(time.0)))
        })
        .hidden(),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
