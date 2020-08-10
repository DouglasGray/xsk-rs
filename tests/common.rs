use rust_xsk::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
};
use std::{io, num::NonZeroU32, thread, time::Duration};

const DEFAULT_IF_NAME: &str = "lo";
const DEFAULT_QUEUE_ID: u32 = 0;

pub struct UmemConfigBuilder {
    pub frame_count: u32,
    pub frame_size: u32,
    pub fill_queue_size: u32,
    pub comp_queue_size: u32,
    pub frame_headroom: u32,
    pub use_huge_pages: bool,
    pub umem_flags: UmemFlags,
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
            umem_flags: UmemFlags::empty(),
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
            self.umem_flags,
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

pub fn build_umem(umem_config: Option<UmemConfig>) -> (Umem, FillQueue, CompQueue) {
    let config = match umem_config {
        Some(cfg) => cfg,
        None => UmemConfigBuilder::default().build(),
    };

    Umem::builder(config)
        .create_mmap()
        .expect("Failed to create mmap area")
        .create_umem()
        .expect("Failed to create umem")
}

pub fn build_socket_and_umem_with_retry_on_failure(
    umem_config: Option<UmemConfig>,
    socket_config: Option<SocketConfig>,
    max_retries: u8,
    time_between_attempts: Duration,
) -> ((Umem, FillQueue, CompQueue), (Socket, TxQueue, RxQueue)) {
    let socket_config = match socket_config {
        Some(cfg) => cfg,
        None => SocketConfigBuilder::default().build(),
    };

    let mut attempts = 0;

    loop {
        let (mut umem, fill_q, comp_q) = build_umem(umem_config.clone());

        match Socket::new(
            DEFAULT_IF_NAME,
            DEFAULT_QUEUE_ID,
            socket_config.clone(),
            &mut umem,
        ) {
            Ok(res) => return ((umem, fill_q, comp_q), res),
            Err(e) => {
                attempts += 1;
                if attempts > max_retries {
                    panic!("Max retries hit. Final error: {}", e)
                }
            }
        }
        thread::sleep(time_between_attempts)
    }
}

pub fn build_socket_and_umem(
    umem_config: Option<UmemConfig>,
    socket_config: Option<SocketConfig>,
) -> ((Umem, FillQueue, CompQueue), (Socket, TxQueue, RxQueue)) {
    let socket_config = match socket_config {
        Some(cfg) => cfg,
        None => SocketConfigBuilder::default().build(),
    };

    let (mut umem, fill_q, comp_q) = build_umem(umem_config);

    let (socket, tx_q, rx_q) =
        Socket::new(DEFAULT_IF_NAME, DEFAULT_QUEUE_ID, socket_config, &mut umem)
            .expect("Failed to build socket");

    ((umem, fill_q, comp_q), (socket, tx_q, rx_q))
}
