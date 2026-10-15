//! The detail sections of the kinds of the Network group (plan section 11.3): Service,
//! EndpointSlice, Endpoints, Ingress, IngressClass and NetworkPolicy. The content of each one
//! is the union of Headlamp and Lens (`research/01` section 3.4.3, `research/02` section 4.4).
//!
//! Each object that a network object names is a link: the pod and the node of an endpoint,
//! the Service and the Secret of an Ingress.
//!
//! - `service`: the Service with its ports and its endpoints.
//! - `endpoints`: the endpoints of an EndpointSlice and of the older Endpoints.
//! - `ingress`: the Ingress with its rules and the IngressClass.
//! - `policy`: the NetworkPolicy with its rules in words.

pub(crate) mod endpoints;
pub(crate) mod ingress;
pub(crate) mod policy;
pub(crate) mod service;

/// A port as kubectl prints it: `http 80/TCP`, `443/TCP`.
fn port(name: Option<&str>, number: Option<i32>, protocol: Option<&str>) -> String {
    let name = name.filter(|name| !name.is_empty()).map(|name| format!("{name} ")).unwrap_or_default();
    let number = number.map_or_else(|| "each".to_owned(), |number| number.to_string());
    format!("{name}{number}/{}", protocol.unwrap_or("TCP"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_port_has_its_name_its_number_and_its_protocol() {
        assert_eq!(super::port(Some("http"), Some(80), None), "http 80/TCP");
        assert_eq!(super::port(Some(""), Some(53), Some("UDP")), "53/UDP");
        assert_eq!(super::port(None, None, None), "each/TCP", "a slice port with no number takes each port");
    }
}
