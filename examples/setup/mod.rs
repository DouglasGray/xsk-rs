mod veth_setup;

use std::net::Ipv4Addr;

pub use veth_setup::run_veth_link;

#[derive(Clone)]
pub struct LinkIpAddr {
    addr: Ipv4Addr,
    prefix_len: u8,
}

impl LinkIpAddr {
    pub fn new(addr: Ipv4Addr, prefix_len: u8) -> Self {
        LinkIpAddr { addr, prefix_len }
    }

    pub fn octets(&self) -> [u8; 4] {
        self.addr.octets()
    }
}

#[derive(Clone)]
pub struct VethConfig {
    dev1_if_name: String,
    dev2_if_name: String,
    dev1_addr: [u8; 6],
    dev2_addr: [u8; 6],
    dev1_ip_addr: LinkIpAddr,
    dev2_ip_addr: LinkIpAddr,
}

impl VethConfig {
    pub fn new(
        dev1_if_name: String,
        dev2_if_name: String,
        dev1_addr: [u8; 6],
        dev2_addr: [u8; 6],
        dev1_ip_addr: LinkIpAddr,
        dev2_ip_addr: LinkIpAddr,
    ) -> Self {
        VethConfig {
            dev1_if_name,
            dev2_if_name,
            dev1_addr,
            dev2_addr,
            dev1_ip_addr,
            dev2_ip_addr,
        }
    }

    pub fn dev1_name(&self) -> &str {
        &self.dev1_if_name
    }

    pub fn dev2_name(&self) -> &str {
        &self.dev2_if_name
    }

    pub fn dev1_addr(&self) -> &[u8; 6] {
        &self.dev1_addr
    }

    pub fn dev2_addr(&self) -> &[u8; 6] {
        &self.dev2_addr
    }

    pub fn dev1_ip_addr(&self) -> &LinkIpAddr {
        &self.dev1_ip_addr
    }

    pub fn dev2_ip_addr(&self) -> &LinkIpAddr {
        &self.dev2_ip_addr
    }
}
