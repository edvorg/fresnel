//! The Secret: its description. No store holds the value of a Secret (plan section 9.4).

use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for Secret {
    const ID: Kind = Kind::Secret;
    const TITLE: &'static str = "Secrets";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Type", 200.0, |secret| Value::text(secret.type_.as_deref())),
        Column::text("Keys", 220.0, |secret| Value::list(secret.data.iter().flatten().map(|(key, _)| key.as_str()))),
    ];
    const ACTIONS: &'static [Action] = &[Action::EditKeys, Action::AddKey, Action::RemoveKey];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }

    /// Each value becomes empty, and the keys stay for the column. The view of a Secret
    /// reads its values from the server when the user asks for them.
    fn trim(&mut self) {
        for value in self.data.iter_mut().flat_map(|data| data.values_mut()) {
            *value = ByteString(Vec::new());
        }
        self.string_data = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::Object;

    #[test]
    fn a_secret_in_a_store_keeps_its_keys_and_no_value() {
        let mut secret = Secret {
            data: Some([("password".to_owned(), ByteString(b"not-a-secret".to_vec()))].into()),
            string_data: Some([("token".to_owned(), "not-a-secret".to_owned())].into()),
            ..Secret::default()
        };
        secret.strip();
        let data = secret.data.expect("the keys stay");
        assert_eq!(data.keys().collect::<Vec<_>>(), ["password"]);
        assert!(data.values().all(|value| value.0.is_empty()) && secret.string_data.is_none());
    }
}
