//! The options of each cluster, `[clusters."name"]`, and the cluster groups, `[groups]`.

use std::collections::BTreeMap;

use toml::Spanned;
use toml::de::DeValue;

use super::Reader;
use crate::config::clusters::{
    ALIAS_LIMIT, ClusterGroup, ClusterOptions, ConfirmRule, FAVORITES_LIMIT, OPTION_KEYS, Rgb,
};

/// The most clusters of one group: more than the top bar shows.
const GROUP_LIMIT: usize = 64;
/// The longest name of a namespace in Kubernetes.
const NAMESPACE_LIMIT: usize = 63;

impl Reader<'_> {
    /// The table `[clusters]`: a table of options for each cluster, by the name of its context.
    pub(super) fn clusters(&mut self, value: &Spanned<DeValue<'_>>) -> BTreeMap<String, ClusterOptions> {
        let mut clusters = BTreeMap::new();
        let Some(table) = self.table_of("clusters", value) else {
            return clusters;
        };
        for (name, value) in table.iter() {
            let name = name.get_ref().as_ref();
            let path = format!("clusters.\"{name}\"");
            let Some(entries) = self.table_of(&path, value) else {
                continue;
            };
            let mut options = ClusterOptions::default();
            for (key, value) in entries.iter() {
                self.option(&path, key, value, &mut options);
            }
            clusters.insert(name.to_owned(), options);
        }
        clusters
    }

    fn option(
        &mut self,
        path: &str,
        key: &Spanned<std::borrow::Cow<'_, str>>,
        value: &Spanned<DeValue<'_>>,
        options: &mut ClusterOptions,
    ) {
        let name = key.get_ref().as_ref();
        let path = format!("{path}.{name}");
        match name {
            "alias" => {
                let text = self.text(&path, value).map(str::trim);
                match text.filter(|text| !text.is_empty() && text.chars().count() <= ALIAS_LIMIT) {
                    Some(alias) => options.alias = Some(alias.to_owned()),
                    None if text.is_some() => {
                        self.problem(value.span(), format!("`{path}` has 1 to {ALIAS_LIMIT} characters"));
                    }
                    None => {}
                }
            }
            "color" => {
                let text = self.text(&path, value);
                options.color = text.and_then(Rgb::parse);
                if text.is_some() && options.color.is_none() {
                    self.problem(value.span(), format!("`{path}` is a color such as \"#3fb950\""));
                }
            }
            "read_only" => options.read_only = self.flag(&path, value),
            "default_namespace" => {
                options.default_namespace = self.text(&path, value).and_then(|text| self.namespace(&path, text, value));
            }
            "favorite_namespaces" => {
                let items = self.texts(&path, value, FAVORITES_LIMIT).unwrap_or_default();
                options.favorite_namespaces =
                    items.iter().filter_map(|item| self.namespace(&path, item, value)).collect();
            }
            "confirm" => {
                let text = self.text(&path, value);
                options.confirm = text.and_then(ConfirmRule::named);
                if text.is_some() && options.confirm.is_none() {
                    self.problem(value.span(), format!("`{path}` is \"dialog\" or \"type-name\""));
                }
            }
            other => {
                let known = OPTION_KEYS.join(", ");
                self.problem(key.span(), format!("unknown option `{other}` of a cluster; the options are {known}"));
            }
        }
    }

    /// A name of a namespace, or a problem.
    fn namespace(&mut self, path: &str, text: &str, value: &Spanned<DeValue<'_>>) -> Option<String> {
        let fits = is_namespace(text);
        if !fits {
            self.problem(value.span(), format!("`{path}`: `{text}` is no name of a namespace"));
        }
        fits.then(|| text.to_owned())
    }

    /// The table `[groups]`: a list of the names of clusters for each name of a group.
    pub(super) fn groups(&mut self, value: &Spanned<DeValue<'_>>) -> Vec<ClusterGroup> {
        let Some(table) = self.table_of("groups", value) else {
            return Vec::new();
        };
        let mut groups = Vec::new();
        for (name, value) in table.iter() {
            let name = name.get_ref().as_ref();
            let path = format!("groups.\"{name}\"");
            if let Some(clusters) = self.texts(&path, value, GROUP_LIMIT) {
                groups.push(ClusterGroup { name: name.to_owned(), clusters });
            }
        }
        groups
    }
}

/// True for a name of a namespace: a name of Kubernetes has small letters, digits and `-`,
/// so a typing mistake shows in the settings and not as an empty filter.
pub fn is_namespace(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= NAMESPACE_LIMIT
        && text.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
