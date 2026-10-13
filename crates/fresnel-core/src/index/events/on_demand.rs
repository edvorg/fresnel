//! The Events of one object by a request to the server, for a cluster that keeps no store
//! of the Events (`EventsMode::OnDemand`). The result has the Events that the Event index
//! gives for the same object.

use std::cmp::Reverse;

use k8s_openapi::api::core::v1::Event;
use kube::core::Request;
use kube::core::params::ListParams;

use super::{Subject, last_seen, subject};
use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::store::ObjectKey;
use crate::watch::Page;

/// The most Events of one page of the server.
const PAGE: u32 = 500;

/// The Events of one object, as one read of the server gave them.
#[derive(Clone, Debug, PartialEq)]
pub struct EventsRead {
    /// The Events, the newest first.
    pub events: Vec<Event>,
    /// True when the server has more Events of the object than the read took.
    pub more: bool,
}

/// The request for one page of the Events that name an object of this kind and name. The
/// server cannot select "this UID or no UID", so the app selects by the UID itself.
fn page(kind: Kind, key: &ObjectKey, token: Option<&str>) -> Result<http::Request<Vec<u8>>, CoreError> {
    let events = Kind::Event.info();
    // An object with no namespace has its Events in some namespace: the kubelet writes
    // the Events of its node into `default`.
    let path = if key.namespace.is_empty() { events.path() } else { events.path_in(&key.namespace) };
    let selector = format!("involvedObject.kind={},involvedObject.name={}", kind.info().name, key.name);
    let mut params = ListParams::default().fields(&selector).limit(PAGE);
    if let Some(token) = token {
        params = params.continue_token(token);
    }
    let built = Request::new(path).list(&params);
    built.map_err(|error| CoreError::invalid(format!("the request for the Events cannot be built: {error}")))
}

/// Reads the Events about one object from the server, `max` at most. With more Events
/// than that, the result has the first ones in the sequence of the server, and tells so.
pub async fn read_events(
    gateway: &Gateway,
    kind: Kind,
    key: &ObjectKey,
    uid: &str,
    max: usize,
) -> Result<EventsRead, CoreError> {
    let is_about = |event: &Event| match subject(event) {
        Some(Subject::Uid(named)) => *named == *uid,
        Some(Subject::Name(named_kind, named)) => named_kind == kind && named == *key,
        None => false,
    };
    let (mut events, mut token) = (Vec::new(), None);
    let more = loop {
        let mut read: Page<Event> = gateway.list_page(page(kind, key, token.as_deref())?).await?;
        token = read.next_token(token.as_deref())?;
        events.extend(read.items.unwrap_or_default().into_iter().filter(is_about));
        if token.is_none() || events.len() > max {
            break events.len() > max || token.is_some();
        }
    };
    events.truncate(max);
    events.sort_by_key(|event| (Reverse(last_seen(event)), event.metadata.name.clone()));
    Ok(EventsRead { events, more })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_selects_the_events_of_one_object_by_its_kind_and_name() {
        let uri = |kind: Kind, key: ObjectKey, token: Option<&str>| {
            page(kind, &key, token).expect("a request").uri().to_string()
        };
        let pod = "fieldSelector=involvedObject.kind%3DPod%2CinvolvedObject.name%3Dweb-0&limit=500";
        assert_eq!(
            uri(Kind::Pod, ObjectKey::new("shop", "web-0"), None),
            format!("/api/v1/namespaces/shop/events?&{pod}")
        );
        // The Events of an object with no namespace are in the namespace of their source.
        let node = "fieldSelector=involvedObject.kind%3DNode%2CinvolvedObject.name%3Dnode-1&limit=500&continue=next";
        assert_eq!(uri(Kind::Node, ObjectKey::new("", "node-1"), Some("next")), format!("/api/v1/events?&{node}"));
    }
}
