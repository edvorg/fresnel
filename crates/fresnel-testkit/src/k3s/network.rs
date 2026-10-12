//! The Docker network of one test cluster, with a fixed address for the container.
//!
//! On the default network, a restart can give the container a new address. k3s then stops
//! with "failed to find interface with specified node ip". A network with its own subnet
//! lets the container keep one address.

use std::net::Ipv4Addr;

use super::docker::{docker, docker_blocking};
use crate::KitError;

/// The subnets `10.199.N.0/24`. The default pools of Docker are in other ranges.
const SUBNET_PREFIX: [u8; 2] = [10, 199];
/// The text of Docker for a subnet that a different network uses.
const SUBNET_IN_USE: &str = "overlaps";

/// A Docker network. Its drop removes it.
pub(super) struct Network {
    name: String,
    /// The address for the one container of the network.
    pub(super) address: Ipv4Addr,
}

impl Network {
    /// Tries each subnet one time, from a random start, so that two tests seldom collide.
    pub(super) async fn create(name: &str) -> Result<Self, KitError> {
        let first = fastrand::u8(..);
        for step in 0..=u8::MAX {
            let [a, b] = SUBNET_PREFIX;
            let c = first.wrapping_add(step);
            let subnet = format!("{}/24", Ipv4Addr::new(a, b, c, 0));
            match docker(&["network", "create", "--subnet", &subnet, name]).await {
                Ok(_) => return Ok(Self { name: name.to_owned(), address: Ipv4Addr::new(a, b, c, 2) }),
                Err(error) if error.to_string().contains(SUBNET_IN_USE) => {}
                Err(error) => return Err(error),
            }
        }
        Err(KitError::new("docker network create: each subnet of 10.199.0.0/16 is in use"))
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        if let Err(error) = docker_blocking(&["network", "rm", &self.name]) {
            eprintln!("fresnel-testkit: {error}");
        }
    }
}
