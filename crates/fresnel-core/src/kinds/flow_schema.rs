//! The FlowSchema: its description and its health value. A FlowSchema sorts the requests
//! of the API server into the priority levels of its flow control.

use k8s_openapi::api::flowcontrol::v1::FlowSchema;

use super::describe::by_metadata;
use super::health::{Condition as _, ObjectHealth, condition, is_true};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

/// The condition of a FlowSchema whose priority level does not exist.
const DANGLING: &str = "Dangling";

impl Describe for FlowSchema {
    const ID: Kind = Kind::FlowSchema;
    const TITLE: &'static str = "FlowSchemas";
    const SIDEBAR: Sidebar = Sidebar::Other;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Priority level", 180.0, |schema| {
            Value::text(schema.spec.as_ref().map(|spec| spec.priority_level_configuration.name.as_str()))
        }),
        Column::number("Matching precedence", 90.0, |schema| {
            read::count(schema.spec.as_ref().and_then(|spec| spec.matching_precedence))
        }),
        Column::text("Distinguisher", 110.0, |schema| {
            let method = schema.spec.as_ref().and_then(|spec| spec.distinguisher_method.as_ref());
            Value::text(method.map(|method| method.type_.as_str()))
        }),
        Column::text("Missing priority level", 90.0, |schema| {
            let conditions = schema.status.as_ref().and_then(|status| status.conditions.as_ref());
            Value::text(condition(conditions, DANGLING).map(|dangling| dangling.status()))
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    /// A warning when the priority level of the schema does not exist: the API server then
    /// puts its requests into the level `global-default`.
    fn health(&self) -> ObjectHealth {
        let conditions = self.status.as_ref().and_then(|status| status.conditions.as_ref());
        if is_true(conditions, DANGLING) { ObjectHealth::Warning } else { by_metadata(&self.metadata) }
    }
}
