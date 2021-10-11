use std::num::NonZeroU32;
use std::sync::Arc;
use xsk_rs::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
};

pub struct UmemConfigBuilder {
    pub frame_count: u32,
    pub frame_size: u32,
    pub fill_queue_size: u32,
    pub comp_queue_size: u32,
    pub frame_headroom: u32,
    pub use_huge_pages: bool,
}

impl UmemConfigBuilder {
    pub fn default() -> Self {
        UmemConfigBuilder {
            frame_count: 16,
            frame_size: 2048,
            fill_queue_size: 8,
            comp_queue_size: 8,
            frame_headroom: 0,
            use_huge_pages: false,
        }
    }

    pub fn build(self) -> UmemConfig {
        UmemConfig::new(
            NonZeroU32::new(self.frame_count).unwrap(),
            NonZeroU32::new(self.frame_size).unwrap(),
            self.fill_queue_size,
            self.comp_queue_size,
            self.frame_headroom,
            self.use_huge_pages,
        )
        .unwrap()
    }
}

pub struct SocketConfigBuilder {
    pub rx_queue_size: u32,
    pub tx_queue_size: u32,
    pub libbpf_flags: LibbpfFlags,
    pub xdp_flags: XdpFlags,
    pub bind_flags: BindFlags,
}

impl SocketConfigBuilder {
    pub fn default() -> Self {
        SocketConfigBuilder {
            rx_queue_size: 8,
            tx_queue_size: 8,
            libbpf_flags: LibbpfFlags::empty(),
            xdp_flags: XdpFlags::empty(),
            bind_flags: BindFlags::empty(),
        }
    }

    pub fn build(self) -> SocketConfig {
        SocketConfig::new(
            self.rx_queue_size,
            self.tx_queue_size,
            self.libbpf_flags,
            self.xdp_flags,
            self.bind_flags,
        )
        .unwrap()
    }
}

fn build_umem(umem_config: Option<UmemConfig>) -> (Arc<Umem>, FillQueue, CompQueue, Vec<Frame>) {
    let config = match umem_config {
        Some(cfg) => cfg,
        None => UmemConfigBuilder::default().build(),
    };

    Umem::builder(config)
        .create_mmap()
        .expect("failed to create mmap area")
        .create_umem()
        .expect("failed to create umem")
}

pub fn build_socket_and_umem<'a, 'umem>(
    umem_config: Option<UmemConfig>,
    socket_config: Option<SocketConfig>,
    if_name: &'a str,
    queue_id: u32,
) -> (
    (Arc<Umem>, FillQueue, CompQueue, Vec<Frame>),
    (TxQueue, RxQueue),
) {
    let socket_config = match socket_config {
        Some(cfg) => cfg,
        None => SocketConfigBuilder::default().build(),
    };

    let (umem, fill_q, comp_q, frame_descs) = build_umem(umem_config);

    let (tx_q, rx_q) = Socket::new(socket_config, Arc::clone(&umem), if_name, queue_id)
        .expect("failed to build socket");

    ((umem, fill_q, comp_q, frame_descs), (tx_q, rx_q))
}
