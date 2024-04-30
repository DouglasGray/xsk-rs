mod util;
pub use util::PacketGenerator;

pub mod veth_setup;
pub use veth_setup::{LinkIpAddr, VethDevConfig};

use std::{net::Ipv4Addr, num::NonZeroU32};
use xsk_rs::{
    config::{Interface, SocketConfig, UmemConfig},
    socket::{RxQueue, Socket, TxQueue},
    umem::{frame::FrameDesc, CompQueue, FillQueue, Umem},
};

pub const ETHERNET_PACKET: [u8; 42] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a, 0x08, 0x06, 0x00, 0x01,
    0x08, 0x00, 0x06, 0x04, 0x00, 0x01, 0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a, 0xc0, 0xa8, 0x45, 0x01,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0xa8, 0x45, 0xfe,
];

pub struct Xsk {
    pub umem: Umem,
    pub fq: FillQueue,
    pub cq: CompQueue,
    pub tx_q: TxQueue,
    pub rx_q: RxQueue,
    pub descs: Vec<FrameDesc>,
}

#[derive(Debug, Clone)]
pub struct XskConfig {
    pub frame_count: NonZeroU32,
    pub umem_config: UmemConfig,
    pub socket_config: SocketConfig,
}

pub fn default_veth_dev_configs() -> (VethDevConfig, VethDevConfig) {
    let dev1_config = VethDevConfig::new(
        "xsk_test_dev1".into(),
        Some([0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a]),
        Some(LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24)),
    );

    let dev2_config = VethDevConfig::new(
        "xsk_test_dev2".into(),
        Some([0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31]),
        Some(LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 2), 24)),
    );

    (dev1_config, dev2_config)
}

pub fn build_socket_and_umem(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    frame_count: NonZeroU32,
    if_name: &Interface,
    queue_id: u32,
) -> Xsk {
    let (umem, descs) = Umem::new(umem_config, frame_count, false).expect("failed to build umem");

    let (tx_q, rx_q, fq_and_cq) = unsafe {
        Socket::new(socket_config, &umem, if_name, queue_id).expect("failed to build socket")
    };

    let (fq, cq) = fq_and_cq.expect(&format!(
        "missing fill and comp queue - interface {:?} may already be bound to",
        if_name
    ));

    Xsk {
        umem,
        fq,
        cq,
        tx_q,
        rx_q,
        descs,
    }
}

pub async fn run_test<F>(xsk1_config: XskConfig, xsk2_config: XskConfig, test: F)
where
    F: Fn((Xsk, PacketGenerator), (Xsk, PacketGenerator)) + Send + 'static,
{
    let (dev1_config, dev2_config) = default_veth_dev_configs();

    let inner = move |dev1_config: VethDevConfig, dev2_config: VethDevConfig| {
        let xsk1 = build_socket_and_umem(
            xsk1_config.umem_config,
            xsk1_config.socket_config,
            xsk1_config.frame_count,
            &dev1_config
                .if_name()
                .parse()
                .expect("failed to parse interface name"),
            0,
        );

        let xsk2 = build_socket_and_umem(
            xsk2_config.umem_config,
            xsk2_config.socket_config,
            xsk2_config.frame_count,
            &dev2_config
                .if_name()
                .parse()
                .expect("failed to parse interface name"),
            0,
        );

        let dev1_pkt_gen = PacketGenerator::new(dev1_config, dev2_config);
        let dev2_pkt_gen = dev1_pkt_gen.clone().into_swapped();

        test((xsk1, dev1_pkt_gen), (xsk2, dev2_pkt_gen))
    };

    veth_setup::run_with_veth_pair(inner, dev1_config, dev2_config)
        .await
        .unwrap();
}

pub async fn run_test_with_dev_configs<F>(
    xsk1_configs: (XskConfig, VethDevConfig),
    xsk2_configs: (XskConfig, VethDevConfig),
    test: F,
) where
    F: Fn((Xsk, PacketGenerator), (Xsk, PacketGenerator)) + Send + 'static,
{
    let (xsk1_config, dev1_config) = xsk1_configs;
    let (xsk2_config, dev2_config) = xsk2_configs;

    let inner = move |dev1_config: VethDevConfig, dev2_config: VethDevConfig| {
        let xsk1 = build_socket_and_umem(
            xsk1_config.umem_config,
            xsk1_config.socket_config,
            xsk1_config.frame_count,
            &dev1_config
                .if_name()
                .parse()
                .expect("failed to parse interface name"),
            0,
        );

        let xsk2 = build_socket_and_umem(
            xsk2_config.umem_config,
            xsk2_config.socket_config,
            xsk2_config.frame_count,
            &dev2_config
                .if_name()
                .parse()
                .expect("failed to parse interface name"),
            0,
        );

        let dev1_pkt_gen = PacketGenerator::new(dev1_config, dev2_config);
        let dev2_pkt_gen = dev1_pkt_gen.clone().into_swapped();

        test((xsk1, dev1_pkt_gen), (xsk2, dev2_pkt_gen))
    };

    veth_setup::run_with_veth_pair(inner, dev1_config, dev2_config)
        .await
        .unwrap();
}
