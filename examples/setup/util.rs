use crossbeam_channel::{self, Receiver};
use etherparse::PacketBuilder;

use super::VethConfig;

pub fn ctrl_channel() -> Result<Receiver<()>, ctrlc::Error> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })?;

    Ok(rx)
}

fn generate_random_bytes(len: u32) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}

// Generate an ETH frame w/ UDP as transport layer and payload size `payload_len`
pub fn generate_eth_frame(veth_config: &VethConfig, payload_len: u32) -> Vec<u8> {
    let builder = PacketBuilder::ethernet2(
        veth_config.dev1_addr().clone(), // src mac
        veth_config.dev2_addr().clone(), // dst mac
    )
    .ipv4(
        veth_config.dev1_ip_addr().octets(), // src ip
        veth_config.dev2_ip_addr().octets(), // dst ip
        20,                                  // time to live
    )
    .udp(
        1234, // src port
        1234, // dst port
    );

    let payload = generate_random_bytes(payload_len);

    let mut result = Vec::<u8>::with_capacity(builder.size(payload.len()));

    builder.write(&mut result, &payload).unwrap();

    result
}
