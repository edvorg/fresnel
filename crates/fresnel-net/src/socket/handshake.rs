//! The WebSocket handshake of an upgrade request (RFC 6455, section 4) with its subprotocols.
//!
//! The caller sends the request with the headers of an [`Offer`] and checks the status of
//! the answer itself, so that it reads the message of a refusal (kube-rs drops it). This
//! module checks the answer HTTP 101: its headers and a subprotocol of the offer.

use http::header::{
    CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_PROTOCOL, SEC_WEBSOCKET_VERSION,
};
use http::header::{HeaderMap, HeaderName, HeaderValue, UPGRADE};
use http::{Response, StatusCode};
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;

use crate::error::{ErrorClass, NetError};

/// The key and the subprotocols of one upgrade request.
#[derive(Clone, Debug)]
pub struct Offer {
    key: String,
    protocols: &'static [&'static str],
}

impl Offer {
    /// A new random key, and the subprotocols with the preferred one first.
    pub fn new(protocols: &'static [&'static str]) -> Self {
        Self { key: generate_key(), protocols }
    }

    /// Sets the headers of the handshake on the request. The API server reads the list of
    /// the subprotocols from one header with commas: it does not join two headers.
    pub fn apply(&self, headers: &mut HeaderMap) -> Result<(), NetError> {
        let value = |text: &str| {
            HeaderValue::from_str(text).map_err(|_| NetError::invalid(format!("the header text {text:?} is not valid")))
        };
        headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
        headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(SEC_WEBSOCKET_VERSION, HeaderValue::from_static("13"));
        headers.insert(SEC_WEBSOCKET_KEY, value(&self.key)?);
        headers.insert(SEC_WEBSOCKET_PROTOCOL, value(&self.protocols.join(", "))?);
        Ok(())
    }

    /// Checks an answer HTTP 101 and gives the subprotocol that the server selected.
    pub(crate) fn accept<B>(&self, response: &Response<B>) -> Result<&'static str, NetError> {
        let not_a_socket = |what: &str| {
            NetError::new(ErrorClass::Server, format!("the answer to the upgrade is not a WebSocket: {what}"))
        };
        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            return Err(not_a_socket(&format!("HTTP {}", response.status().as_u16())));
        }
        let headers = response.headers();
        if !has(headers, &UPGRADE, |text| text.eq_ignore_ascii_case("websocket")) {
            return Err(not_a_socket("no header `Upgrade: websocket`"));
        }
        let upgrade = |text: &str| text.split(',').any(|part| part.trim().eq_ignore_ascii_case("upgrade"));
        if !has(headers, &CONNECTION, upgrade) {
            return Err(not_a_socket("no header `Connection: Upgrade`"));
        }
        let accept = derive_accept_key(self.key.as_bytes());
        if !has(headers, &SEC_WEBSOCKET_ACCEPT, |text| text == accept) {
            return Err(not_a_socket("the header `Sec-WebSocket-Accept` does not fit the key of the request"));
        }
        let selected = headers.get(SEC_WEBSOCKET_PROTOCOL).and_then(|value| value.to_str().ok()).unwrap_or_default();
        self.protocols.iter().copied().find(|protocol| *protocol == selected).ok_or_else(|| {
            let offered = self.protocols.join(", ");
            NetError::new(
                ErrorClass::Server,
                format!("the server selected the subprotocol {selected:?}, not one of {offered}"),
            )
        })
    }
}

fn has(headers: &HeaderMap, name: &HeaderName, fits: impl Fn(&str) -> bool) -> bool {
    headers.get(name).and_then(|value| value.to_str().ok()).is_some_and(fits)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROTOCOLS: &[&str] = &["v5.channel.k8s.io", "v4.channel.k8s.io"];

    /// The answer of a server that selected `protocol`, with the accept key for `key`.
    fn answer(key: &str, protocol: Option<&str>) -> Response<()> {
        let mut response = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(CONNECTION, "Upgrade")
            .header(UPGRADE, "websocket")
            .header(SEC_WEBSOCKET_ACCEPT, derive_accept_key(key.as_bytes()));
        if let Some(protocol) = protocol {
            response = response.header(SEC_WEBSOCKET_PROTOCOL, protocol);
        }
        response.body(()).expect("an answer")
    }

    #[test]
    fn the_request_offers_each_subprotocol_in_one_header() {
        let offer = Offer::new(PROTOCOLS);
        let mut headers = HeaderMap::new();
        offer.apply(&mut headers).expect("the headers");
        assert_eq!(headers[SEC_WEBSOCKET_PROTOCOL], "v5.channel.k8s.io, v4.channel.k8s.io");
        assert_eq!(
            (&headers[UPGRADE], &headers[CONNECTION], &headers[SEC_WEBSOCKET_VERSION]),
            (
                &HeaderValue::from_static("websocket"),
                &HeaderValue::from_static("Upgrade"),
                &HeaderValue::from_static("13")
            )
        );
        assert_eq!(headers[SEC_WEBSOCKET_KEY], offer.key.as_str());
        assert_ne!(Offer::new(PROTOCOLS).key, offer.key, "each request has a key of its own");
    }

    #[test]
    fn an_answer_gives_the_subprotocol_that_the_server_selected() {
        let offer = Offer::new(PROTOCOLS);
        assert_eq!(offer.accept(&answer(&offer.key, Some("v4.channel.k8s.io"))), Ok("v4.channel.k8s.io"));
        assert_eq!(offer.accept(&answer(&offer.key, Some("v5.channel.k8s.io"))), Ok("v5.channel.k8s.io"));
    }

    #[test]
    fn an_answer_that_is_not_a_websocket_of_the_offer_is_refused_with_its_reason() {
        let offer = Offer::new(PROTOCOLS);
        let refused = |response: Response<()>| {
            let error = offer.accept(&response).expect_err("refused");
            assert_eq!(error.class(), ErrorClass::Server);
            error.to_string()
        };
        let start = "the answer to the upgrade is not a WebSocket: ";
        assert_eq!(
            refused(answer("an other key", Some(PROTOCOLS[0]))),
            format!("{start}the header `Sec-WebSocket-Accept` does not fit the key of the request")
        );
        let mut ok = answer(&offer.key, Some(PROTOCOLS[0]));
        *ok.status_mut() = StatusCode::OK;
        assert_eq!(refused(ok), format!("{start}HTTP 200"));
        let mut plain = answer(&offer.key, Some(PROTOCOLS[0]));
        plain.headers_mut().remove(UPGRADE);
        assert_eq!(refused(plain), format!("{start}no header `Upgrade: websocket`"));
        assert_eq!(
            refused(answer(&offer.key, None)),
            "the server selected the subprotocol \"\", not one of v5.channel.k8s.io, v4.channel.k8s.io"
        );
        assert_eq!(
            refused(answer(&offer.key, Some("v3.channel.k8s.io"))),
            "the server selected the subprotocol \"v3.channel.k8s.io\", not one of v5.channel.k8s.io, v4.channel.k8s.io"
        );
    }
}
