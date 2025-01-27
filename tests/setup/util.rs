use etherparse::{err::packet::BuildWriteError, PacketBuilder};

use super::veth_setup::VethDevConfig;

#[derive(Debug, Clone)]
pub struct PacketGenerator {
    src: VethDevConfig,
    dst: VethDevConfig,
}

impl PacketGenerator {
    pub fn new(src: VethDevConfig, dst: VethDevConfig) -> Self {
        Self { src, dst }
    }

    /// Generate an ETH frame w/ UDP as transport layer and payload size `payload_len`
    pub fn generate_packet(
        &self,
        src_port: u16,
        dst_port: u16,
        payload_len: usize,
    ) -> Result<Vec<u8>, BuildWriteError> {
        let builder = PacketBuilder::ethernet2(
            self.src.addr().unwrap(), // src mac
            self.dst.addr().unwrap(), // dst mac
        )
        .ipv4(
            self.src.ip_addr().unwrap().octets(), // src ip
            self.dst.ip_addr().unwrap().octets(), // dst ip
            20,                                   // time to live
        )
        .udp(src_port, dst_port);

        let payload = generate_random_bytes(payload_len);

        let mut result = Vec::with_capacity(builder.size(payload.len()));

        builder.write(&mut result, &payload)?;

        Ok(result)
    }

    /// Packet generator with `src` and `dst` swapped.
    pub fn into_swapped(self) -> Self {
        Self {
            src: self.dst.clone(),
            dst: self.src.clone(),
        }
    }
}

fn generate_random_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}
