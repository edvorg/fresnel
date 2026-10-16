//! A forward rule: what the user asked for, and what the settings file saves.

use std::fmt::{self, Display};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::cluster::ClusterId;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// The kinds that a forward can reach (plan section 12.3): a pod, a Service through its
/// EndpointSlices, and each workload through its pods.
pub const TARGETS: [Kind; 8] = [
    Kind::Pod,
    Kind::Service,
    Kind::Deployment,
    Kind::StatefulSet,
    Kind::DaemonSet,
    Kind::ReplicaSet,
    Kind::ReplicationController,
    Kind::Job,
];

/// The local port of a forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LocalPort {
    /// This port, or the state "failed": a fixed port never changes silently.
    Fixed(u16),
    /// A free port of the system, the same on each address, for the life of the forward.
    Automatic,
}

impl LocalPort {
    /// The port of the bind: 0 lets the system choose.
    pub fn number(self) -> u16 {
        match self {
            Self::Fixed(port) => port,
            Self::Automatic => 0,
        }
    }
}

/// The local addresses of a forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bind {
    /// `127.0.0.1` and `::1`, as kubectl: only programs of this computer reach the port.
    Localhost,
    /// `0.0.0.0` and `::`: each computer that reaches this one reaches the port too.
    Everywhere,
    /// One address of an interface of this computer.
    Address(IpAddr),
}

impl Bind {
    /// The text of the settings file: "localhost", "everywhere" or an address.
    pub fn text(self) -> String {
        match self {
            Self::Localhost => "localhost".to_owned(),
            Self::Everywhere => "everywhere".to_owned(),
            Self::Address(address) => address.to_string(),
        }
    }

    /// The bind of a text of the settings file or of the dialog.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "localhost" => Some(Self::Localhost),
            "everywhere" => Some(Self::Everywhere),
            other => other.parse().ok().map(Self::Address),
        }
    }

    /// The addresses of the listener.
    pub fn addresses(self) -> Vec<IpAddr> {
        match self {
            Self::Localhost => vec![IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)],
            Self::Everywhere => vec![IpAddr::V4(Ipv4Addr::UNSPECIFIED), IpAddr::V6(Ipv6Addr::UNSPECIFIED)],
            Self::Address(address) => vec![address],
        }
    }
}

/// One forward: a port of an object of a cluster, and its local side. Two forwards with an
/// equal rule are one forward.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ForwardRule {
    pub cluster: ClusterId,
    /// One kind of [`TARGETS`].
    pub kind: Kind,
    /// The namespace and the name of the object.
    pub key: ObjectKey,
    /// A port of a container of the pods, or a port of the Service.
    pub port: u16,
    pub local: LocalPort,
    pub bind: Bind,
}

impl ForwardRule {
    /// Why the app cannot run the rule, if it cannot.
    pub fn problem(&self) -> Option<String> {
        if !TARGETS.contains(&self.kind) {
            return Some(format!("a forward reaches no {}", self.kind.info().name));
        }
        if self.key.namespace.is_empty() || self.key.name.is_empty() {
            return Some("a forward needs the namespace and the name of its object".to_owned());
        }
        (self.port == 0).then(|| "a forward needs a port of its object, from 1 to 65535".to_owned())
    }

    /// "Service shop/web port 80": the target of the rule in a text.
    pub fn target(&self) -> String {
        let ObjectKey { namespace, name } = &self.key;
        format!("{} {namespace}/{name} port {}", self.kind.info().name, self.port)
    }
}

impl Display for ForwardRule {
    /// "Service shop/web port 80 in ams-prod".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} in {}", self.target(), self.cluster)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(kind: Kind, port: u16) -> ForwardRule {
        ForwardRule {
            cluster: ClusterId::new("ams"),
            kind,
            key: ObjectKey::new("shop", "web"),
            port,
            local: LocalPort::Automatic,
            bind: Bind::Localhost,
        }
    }

    #[test]
    fn a_rule_names_its_target_and_tells_what_it_lacks() {
        assert_eq!(rule(Kind::Service, 80).to_string(), "Service shop/web port 80 in ams");
        assert_eq!(rule(Kind::Deployment, 8080).problem(), None);
        assert_eq!(rule(Kind::ConfigMap, 80).problem().as_deref(), Some("a forward reaches no ConfigMap"));
        assert!(rule(Kind::Pod, 0).problem().is_some_and(|text| text.contains("port")));
        let nameless = ForwardRule { key: ObjectKey::new("", "web"), ..rule(Kind::Pod, 80) };
        assert!(nameless.problem().is_some_and(|text| text.contains("namespace")));
    }

    #[test]
    fn a_bind_reads_its_text_and_gives_its_addresses() {
        for bind in [Bind::Localhost, Bind::Everywhere, Bind::Address(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)))] {
            assert_eq!(Bind::parse(&bind.text()), Some(bind));
        }
        assert_eq!(Bind::parse("::1"), Some(Bind::Address(IpAddr::V6(Ipv6Addr::LOCALHOST))));
        assert_eq!(Bind::parse("local"), None);
        assert_eq!(Bind::Localhost.addresses(), [IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)]);
        assert_eq!((LocalPort::Automatic.number(), LocalPort::Fixed(8080).number()), (0, 8080));
    }
}
