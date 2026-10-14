//! Where a watch loop reads the objects of one kind from.
//!
//! The loop knows two requests: one page of a list, and a watch. [`GatewaySource`] sends
//! them through the gateway of a cluster. A test of the loop gives a source with a script,
//! so that it can set each answer and use paused time.

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ListMeta;
use kube::core::Request;
use kube::core::params::{ListParams, WatchParams};
use serde::Deserialize;

use crate::cluster::{Gateway, WatchEvent, WatchStream};
use crate::error::CoreError;
use crate::kinds::{KindInfo, Object};

/// One page of a list. The metadata holds the resource version of the list and the token
/// for the next page.
#[derive(Debug, Deserialize)]
pub(crate) struct Page<T> {
    #[serde(default)]
    pub(crate) metadata: ListMeta,
    /// `None` for a list with no object: a kube-apiserver writes `null` then, for the
    /// metadata form of each kind.
    pub(crate) items: Option<Vec<T>>,
}

impl<T> Page<T> {
    /// Takes the token for the next page out of the page. `None` after the last page.
    /// `asked` is the token of the request of this page: a server that gives it back
    /// would get that request again with no pause and with no end.
    pub(crate) fn next_token(&mut self, asked: Option<&str>) -> Result<Option<String>, CoreError> {
        let next = self.metadata.continue_.take().filter(|next| !next.is_empty());
        if next.is_some() && next.as_deref() == asked {
            return Err(CoreError::invalid("the list of the server gave the token of a page two times"));
        }
        Ok(next)
    }
}

/// The requests of one watch loop.
pub(crate) trait Source<T>: Send + Sync + 'static {
    /// The events of one watch.
    type Events: Events<T>;

    /// Reads one page of `limit` objects at most. `token` is the token of the page before.
    fn list_page(&self, limit: u32, token: Option<&str>) -> impl Future<Output = Result<Page<T>, CoreError>> + Send;

    /// Starts a watch after the resource version `from`. The server ends it after
    /// `timeout_seconds`. The result comes after the good status of the server.
    fn watch(&self, from: &str, timeout_seconds: u32) -> impl Future<Output = Result<Self::Events, CoreError>> + Send;
}

/// The events of one watch. A caller can drop a wait for the next event at each moment.
pub(crate) trait Events<T>: Send {
    /// The next event. `None` is the end of the watch.
    fn next(&mut self) -> impl Future<Output = Option<Result<WatchEvent<T>, CoreError>>> + Send;
}

impl<T: Object> Events<T> for WatchStream<T> {
    fn next(&mut self) -> impl Future<Output = Option<Result<WatchEvent<T>, CoreError>>> + Send {
        WatchStream::next(self)
    }
}

/// The requests for one kind of one cluster.
pub(crate) struct GatewaySource {
    gateway: Gateway,
    /// The path of the kind for all namespaces.
    request: Request,
    /// True for the store of a tier B kind that is always on: the server sends the
    /// metadata of each object only.
    metadata_only: bool,
    /// The field selector of each request. The Events store can ask for warnings only.
    fields: Option<&'static str>,
}

impl GatewaySource {
    pub(crate) fn new(gateway: Gateway, kind: &KindInfo, metadata_only: bool, fields: Option<&'static str>) -> Self {
        Self { gateway, request: Request::new(kind.path()), metadata_only, fields }
    }
}

impl<T: Object> Source<T> for GatewaySource {
    type Events = WatchStream<T>;

    async fn list_page(&self, limit: u32, token: Option<&str>) -> Result<Page<T>, CoreError> {
        let mut params = ListParams::default().limit(limit);
        if let Some(token) = token {
            params = params.continue_token(token);
        }
        if let Some(fields) = self.fields {
            params = params.fields(fields);
        }
        let request = if self.metadata_only { self.request.list_metadata(&params) } else { self.request.list(&params) };
        self.gateway.list_page(request.map_err(|error| not_built(&error))?).await
    }

    async fn watch(&self, from: &str, timeout_seconds: u32) -> Result<WatchStream<T>, CoreError> {
        // The default of these parameters asks for bookmarks.
        let mut params = WatchParams::default().timeout(timeout_seconds);
        if let Some(fields) = self.fields {
            params = params.fields(fields);
        }
        let request = if self.metadata_only {
            self.request.watch_metadata(&params, from)
        } else {
            self.request.watch(&params, from)
        };
        self.gateway.watch_stream(request.map_err(|error| not_built(&error))?).await
    }
}

fn not_built(error: &kube::core::request::Error) -> CoreError {
    CoreError::invalid(format!("the request of the store cannot be built: {error}"))
}

#[cfg(test)]
mod tests {
    use http::header::ACCEPT;

    use super::*;
    use crate::kinds::Kind;

    /// On k3s, the metadata list of each kind with no object has `"items":null`. Before
    /// this test, the store of such a kind failed with each list.
    #[test]
    fn a_page_with_no_object_has_null_or_an_empty_list_as_its_items() {
        let page = |items: &str| {
            let text = format!(
                r#"{{"kind":"PartialObjectMetadataList","metadata":{{"resourceVersion":"8"}},"items":{items}}}"#
            );
            serde_json::from_str::<Page<crate::kinds::Meta>>(&text).expect("a page")
        };
        for items in ["null", "[]"] {
            let page = page(items);
            assert_eq!(
                (page.items.unwrap_or_default().len(), page.metadata.resource_version.as_deref()),
                (0, Some("8"))
            );
        }
        assert_eq!(page(r#"[{"metadata":{"name":"a"}}]"#).items.map(|items| items.len()), Some(1));
    }

    /// The request of a tier B kind asks for the metadata form: the data of a Secret must
    /// not arrive in the app.
    #[test]
    fn the_requests_of_a_tier_b_kind_ask_for_the_metadata_form() {
        let secrets = Request::new(Kind::Secret.info().path());
        let list = secrets.list_metadata(&ListParams::default().limit(500)).expect("a request");
        assert_eq!(list.uri(), "/api/v1/secrets?&limit=500");
        assert_eq!(list.headers()[ACCEPT], "application/json;as=PartialObjectMetadataList;g=meta.k8s.io;v=v1");

        let watch = secrets.watch_metadata(&WatchParams::default().timeout(75), "42").expect("a request");
        let query = watch.uri().query().expect("a query");
        for part in ["watch=true", "resourceVersion=42", "timeoutSeconds=75", "allowWatchBookmarks=true"] {
            assert!(query.split('&').any(|pair| pair == part), "{part} in {query}");
        }
        assert_eq!(watch.headers()[ACCEPT], "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1");
    }
}
