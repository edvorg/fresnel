//! The network change detector: the interface addresses and the default route.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

use nix::ifaddrs::getifaddrs;
use nix::net::if_::InterfaceFlags;

use super::{Detector, Disturbance};

/// Addresses for documents. No packet goes to them: the connect of a UDP socket only asks
/// the kernel for the route.
const FAR_V4: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9);
const FAR_V6: SocketAddr = SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)), 9);

/// What the detector compares between two looks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct NetworkFacts {
    /// Each address of each interface that is up, with the name of the interface.
    addresses: BTreeSet<(String, IpAddr)>,
    /// The source address of a packet to a far IPv4 address. It changes with the default route.
    route_v4: Option<IpAddr>,
    /// The same for IPv6.
    route_v6: Option<IpAddr>,
}

/// Detects a change of the interface addresses or of the default route: Wi-Fi to cable, a
/// VPN that starts, a new network.
pub struct NetworkChange {
    read: Box<dyn FnMut() -> NetworkFacts + Send>,
    last: Option<NetworkFacts>,
}

impl NetworkChange {
    /// A detector on the interfaces of the system.
    pub fn system() -> Self {
        Self::new(read_system)
    }

    /// A detector on `read`. A test gives its own facts.
    fn new(read: impl FnMut() -> NetworkFacts + Send + 'static) -> Self {
        Self { read: Box::new(read), last: None }
    }
}

impl Detector for NetworkChange {
    fn look(&mut self) -> Option<Disturbance> {
        let now = (self.read)();
        let before = self.last.replace(now);
        // The first look has nothing to compare.
        (before.is_some() && before != self.last).then_some(Disturbance::NetworkChange)
    }
}

impl std::fmt::Debug for NetworkChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkChange").field("last", &self.last).finish_non_exhaustive()
    }
}

fn read_system() -> NetworkFacts {
    // With an error of the system, the list is empty. If it was not empty before, that is
    // a change, and the app tests its connections.
    let addresses = getifaddrs().into_iter().flatten().filter_map(|interface| {
        let up = interface.flags.contains(InterfaceFlags::IFF_UP | InterfaceFlags::IFF_RUNNING);
        let address = interface.address?;
        let v4 = address.as_sockaddr_in().map(|v4| IpAddr::V4(v4.ip()));
        let v6 = address.as_sockaddr_in6().map(|v6| IpAddr::V6(v6.ip()));
        let ip = v4.or(v6).filter(|ip| up && !ip.is_loopback())?;
        Some((interface.interface_name, ip))
    });
    NetworkFacts { addresses: addresses.collect(), route_v4: source_address(FAR_V4), route_v6: source_address(FAR_V6) }
}

/// The address that the kernel selects for a packet to `far`. `None` with no route.
fn source_address(far: SocketAddr) -> Option<IpAddr> {
    let any: SocketAddr = match far {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let socket = UdpSocket::bind(any).ok()?;
    socket.connect(far).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn facts(addresses: &[(&str, [u8; 4])], route: Option<[u8; 4]>) -> NetworkFacts {
        NetworkFacts {
            addresses: addresses.iter().map(|(name, ip)| ((*name).to_owned(), IpAddr::from(*ip))).collect(),
            route_v4: route.map(IpAddr::from),
            route_v6: None,
        }
    }

    #[test]
    fn a_new_address_and_a_new_route_are_each_one_change() {
        let wifi = facts(&[("en0", [192, 168, 1, 20])], Some([192, 168, 1, 20]));
        let with_vpn = facts(&[("en0", [192, 168, 1, 20]), ("utun4", [10, 8, 0, 2])], Some([192, 168, 1, 20]));
        let vpn_route = facts(&[("en0", [192, 168, 1, 20]), ("utun4", [10, 8, 0, 2])], Some([10, 8, 0, 2]));
        let offline = facts(&[], None);
        let steps =
            [wifi.clone(), wifi.clone(), with_vpn, vpn_route.clone(), vpn_route, offline.clone(), offline, wifi];
        let script = Arc::new(Mutex::new(steps.into_iter()));
        let mut detector = NetworkChange::new(move || script.lock().expect("a lock").next().expect("a scripted look"));

        let seen: Vec<bool> = (0..8).map(|_| detector.look().is_some()).collect();
        assert_eq!(seen, [false, false, true, true, false, true, false, true]);
    }

    #[test]
    fn the_system_read_gives_the_same_facts_two_times() {
        let mut detector = NetworkChange::system();
        assert_eq!(detector.look(), None);
        assert_eq!(detector.look(), None, "nothing changed between two looks");
        assert!(read_system().addresses.iter().all(|(_, ip)| !ip.is_loopback()));
    }
}
