pub mod net;
pub mod util;

use xsk_rs::{CompQueue, FillQueue, RxQueue, TxQueue, Umem};

pub use net::NetConfig;

#[derive(Debug)]
pub enum Role {
    Tx,
    Rx,
}

#[derive(Debug, Clone)]
pub struct Config {
    if_name: String,
    if_queue: u32,
    use_need_wakeup: bool,
    zerocopy: bool,
    drv_mode: bool,
    num_frames_to_process: u32,
    tx_q_size: u32,
    rx_q_size: u32,
    fill_q_size: u32,
    comp_q_size: u32,
    frame_size: u32,
    poll_ms_timeout: i32,
    pkt_payload_len: u32,
    max_batch_size: u32,
}

impl Config {
    pub fn new(
        if_name: String,
        if_queue: u32,
        use_need_wakeup: bool,
        zerocopy: bool,
        drv_mode: bool,
        num_frames_to_process: u32,
    ) -> Self {
        Config {
            if_name,
            if_queue,
            use_need_wakeup,
            zerocopy,
            drv_mode,
            num_frames_to_process,
            tx_q_size: 4096,
            rx_q_size: 4096,
            fill_q_size: 4096 * 2,
            comp_q_size: 4096,
            frame_size: 2048,
            poll_ms_timeout: 100,
            pkt_payload_len: 32,
            max_batch_size: 64,
        }
    }

    pub fn if_name(&self) -> &str {
        &self.if_name
    }

    pub fn if_queue(&self) -> &u32 {
        &self.if_queue
    }

    pub fn use_need_wakeup(&self) -> &bool {
        &self.use_need_wakeup
    }

    pub fn zerocopy(&self) -> &bool {
        &self.zerocopy
    }

    pub fn drv_mode(&self) -> &bool {
        &self.drv_mode
    }

    pub fn num_frames_to_process(&self) -> &u32 {
        &self.num_frames_to_process
    }

    pub fn tx_q_size(&self) -> &u32 {
        &self.tx_q_size
    }

    pub fn rx_q_size(&self) -> &u32 {
        &self.rx_q_size
    }

    pub fn fill_q_size(&self) -> &u32 {
        &self.fill_q_size
    }

    pub fn comp_q_size(&self) -> &u32 {
        &self.comp_q_size
    }

    pub fn poll_ms_timeout(&self) -> &i32 {
        &self.poll_ms_timeout
    }

    pub fn pkt_payload_len(&self) -> &u32 {
        &self.pkt_payload_len
    }

    pub fn max_batch_size(&self) -> &u32 {
        &self.max_batch_size
    }
}

pub struct XskState<'umem> {
    tx_q: TxQueue<'umem>,
    rx_q: RxQueue<'umem>,
    fill_q: FillQueue<'umem>,
    comp_q: CompQueue<'umem>,
    umem: Umem<'umem>,
}

impl<'umem> XskState<'umem> {
    pub fn tx_q(&mut self) -> &mut TxQueue<'umem> {
        &mut self.tx_q
    }

    pub fn rx_q(&mut self) -> &mut RxQueue<'umem> {
        &mut self.rx_q
    }

    pub fn fill_q(&mut self) -> &mut FillQueue<'umem> {
        &mut self.fill_q
    }

    pub fn comp_q(&mut self) -> &mut CompQueue<'umem> {
        &mut self.comp_q
    }

    pub fn umem(&mut self) -> &mut Umem<'umem> {
        &mut self.umem
    }
}
