use etherparse::PacketBuilder;

#[derive(Debug, Clone)]
pub struct NetConfig {
    src_addr: [u8; 6],
    dst_addr: [u8; 6],
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
}

impl NetConfig {
    pub fn new(
        src_addr: [u8; 6],
        dst_addr: [u8; 6],
        src_ip: [u8; 4],
        dst_ip: [u8; 4],
        src_port: u16,
        dst_port: u16,
    ) -> Self {
        NetConfig {
            src_addr,
            dst_addr,
            src_ip,
            dst_ip,
            src_port,
            dst_port,
        }
    }

    pub fn dst_addr(&self) -> &[u8; 6] {
        &self.dst_addr
    }

    pub fn src_addr(&self) -> &[u8; 6] {
        &self.src_addr
    }

    pub fn dst_ip(&self) -> &[u8; 4] {
        &self.dst_ip
    }

    pub fn src_ip(&self) -> &[u8; 4] {
        &self.src_ip
    }

    pub fn dst_port(&self) -> &u16 {
        &self.dst_port
    }

    pub fn src_port(&self) -> &u16 {
        &self.src_port
    }
}

fn generate_random_bytes(len: u32) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}

// Generate an ETH frame w/ UDP as transport layer and payload size `payload_len`
pub fn generate_eth_frame(net_config: &NetConfig, pkt_payload_len: u32) -> Vec<u8> {
    let builder = PacketBuilder::ethernet2(
        net_config.src_addr.clone(), // src mac
        net_config.dst_addr.clone(), // dst mac
    )
    .ipv4(
        net_config.src_ip.clone(), // src ip
        net_config.dst_ip.clone(), // dst ip
        20,                        // time to live
    )
    .udp(
        net_config.src_port, // src port
        net_config.dst_port, // dst port
    );

    let payload = generate_random_bytes(pkt_payload_len);

    let mut result = Vec::<u8>::with_capacity(builder.size(payload.len()));

    builder.write(&mut result, &payload).unwrap();

    result
}
