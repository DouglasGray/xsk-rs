use crossbeam_channel::{self, Receiver};
use etherparse::{err::packet::BuildWriteError, PacketBuilder};

use super::veth_setup::VethDevConfig;

pub fn ctrl_channel() -> Result<Receiver<()>, ctrlc::Error> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })?;

    Ok(rx)
}

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
            self.src.addr(), // src mac
            self.dst.addr(), // dst mac
        )
        .ipv4(
            self.src.ip_addr().octets(), // src ip
            self.dst.ip_addr().octets(), // dst ip
            20,                          // time to live
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
