use rust_xsk::{
    mem::{CompQueue, Config as UmemConfig, FillQueue},
    xsk::{Config as SocketConfig, RxQueue, TxQueue},
};
use std::{io, thread, time::Duration};

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
            self.frame_count,
            self.frame_size,
            self.fill_queue_size,
            self.comp_queue_size,
            self.frame_headroom,
            self.use_huge_pages,
        )
    }
}

pub struct SocketConfigBuilder {
    pub if_name: String,
    pub queue_id: u32,
    pub rx_queue_size: u32,
    pub tx_queue_size: u32,
}

impl SocketConfigBuilder {
    pub fn default() -> Self {
        SocketConfigBuilder {
            if_name: "lo".into(),
            queue_id: 0,
            rx_queue_size: 8,
            tx_queue_size: 8,
        }
    }

    pub fn build(self) -> SocketConfig {
        SocketConfig::new(
            self.if_name,
            self.queue_id,
            self.rx_queue_size,
            self.tx_queue_size,
        )
    }
}

pub fn build_umem(umem_config: Option<UmemConfig>) -> (Umem, FillQueue, CompQueue) {
    let config = match umem_config {
        Some(cfg) => cfg,
        None => UmemConfigBuilder::default().build(),
    };

    Umem::new(config)
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
) -> Result<((Umem, FillQueue, CompQueue), (Socket, TxQueue, RxQueue)), io::Error> {
    let socket_config = match socket_config {
        Some(cfg) => cfg,
        None => SocketConfigBuilder::default().build(),
    };

    let mut attempts = 0;

    loop {
        let (mut umem, fill_q, comp_q) = build_umem(umem_config.clone());
        match Socket::new(socket_config.clone(), &mut umem) {
            Ok(res) => return Ok(((umem, fill_q, comp_q), res)),
            Err(e) => {
                attempts += 1;
                if attempts > max_retries {
                    return Err(e);
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
        Socket::new(socket_config, &mut umem).expect("Failed to build socket");

    ((umem, fill_q, comp_q), (socket, tx_q, rx_q))
}
