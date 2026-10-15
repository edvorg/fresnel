//! The path to the API that the soak compares the stores with: a connection of its own,
//! apart from the core, so a fault of the network of the core does not reach it.

use std::path::PathBuf;
use std::sync::Arc;

use fresnel_core::CoreError;
use fresnel_core::cluster::{ClusterId, Gateway, Link, Registry};
use fresnel_core::kinds::Kind;
use fresnel_core::kubeconfig;
use fresnel_core::net::env::ProcessEnv;
use fresnel_core::net::task::Tasks;
use fresnel_core::net::{ErrorClass, NetSettings};
use kube::core::Request;
use kube::core::params::ListParams;
use serde::Deserialize;
use tokio::runtime::Handle;

use super::cover::{Listed, Version};

/// The objects of one page that the soak reads: the metadata form of the server.
#[derive(Deserialize)]
struct Page {
    metadata: PageMeta,
    /// A list with no object has `null` here.
    items: Option<Vec<Item>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageMeta {
    resource_version: Option<String>,
    #[serde(rename = "continue")]
    next: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    metadata: ItemMeta,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemMeta {
    name: Option<String>,
    namespace: Option<String>,
    uid: Option<String>,
    resource_version: Option<String>,
}

/// The connection of the comparison. The connections close when it drops.
pub(super) struct Api {
    link: Link,
    _registry: Registry,
}

impl Api {
    /// Connects the context in these kubeconfig files with the environment of the process,
    /// so a credential plugin of the user runs as it runs for `kubectl`.
    pub(super) fn connect(tasks: &Tasks, runtime: Handle, files: &[PathBuf], context: &str) -> Result<Self, String> {
        let registry = Registry::new(tasks.clone(), runtime, ProcessEnv::from_process(), NetSettings::default());
        registry.set_contexts(kubeconfig::load(files).contexts);
        let link = registry.connect(&ClusterId::new(context));
        let link = link.map_err(|error| format!("the connect of the comparison did not start: {error}"))?;
        Ok(Self { link, _registry: registry })
    }

    pub(super) fn gateway(&self) -> Gateway {
        self.link.gateway.clone()
    }
}

/// The objects of one kind, as a list of the API gives them now, in pages of 500.
pub(super) async fn list(gateway: &Gateway, kind: Kind) -> Result<Listed, CoreError> {
    let request = Request::new(kind.info().path());
    let (mut listed, mut params) = (Listed::default(), ListParams::default().limit(500));
    loop {
        let request = request.list_metadata(&params);
        let request =
            request.map_err(|error| CoreError::new(ErrorClass::Invalid, format!("a list request: {error}")))?;
        let page: Page = gateway.list_page(request).await?;
        for item in page.items.unwrap_or_default() {
            let meta = item.metadata;
            let key = (Arc::from(meta.namespace.unwrap_or_default()), Arc::from(meta.name.unwrap_or_default()));
            listed.objects.insert(key, Version::new(meta.uid.as_deref(), meta.resource_version.as_deref()));
        }
        // The version of the first page is the version of the list: the next pages are
        // parts of the same list.
        let version = page.metadata.resource_version.and_then(|text| text.parse().ok());
        listed.version = listed.version.or(version);
        match page.metadata.next.filter(|token| !token.is_empty()) {
            Some(token) => params = params.continue_token(&token),
            None => return Ok(listed),
        }
    }
}
