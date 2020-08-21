use std::thread;
use tokio::{
    runtime::Runtime,
    sync::oneshot::{self, error::TryRecvError},
};
use xsk_rs::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
};

mod veth_setup;
mod xsk_setup;

pub use xsk_setup::{SocketConfigBuilder, UmemConfigBuilder};

pub struct SocketState<'umem> {
    pub if_name: String,
    pub umem: Umem<'umem>,
    pub fill_q: FillQueue<'umem>,
    pub comp_q: CompQueue<'umem>,
    pub tx_q: TxQueue<'umem>,
    pub rx_q: RxQueue<'umem>,
    pub frame_descs: Vec<FrameDesc>,
}

pub fn run_bench<'a, 'b, F>(
    umem_config: Option<UmemConfig>,
    socket_config: Option<SocketConfig>,
    mut bench_fn: F,
) where
    F: FnMut(SocketState<'a>, SocketState<'b>),
{
    let dev1_if_name = String::from("xsk_bench_dev1");
    let dev2_if_name = String::from("xsk_bench_dev2");

    let (startup_w, mut startup_r) = oneshot::channel();
    let (shutdown_w, shutdown_r) = oneshot::channel();

    let dev1_if_name_clone = dev1_if_name.clone();
    let dev2_if_name_clone = dev2_if_name.clone();

    let veth_handle = thread::spawn(move || {
        let mut runtime = Runtime::new().unwrap();

        runtime.block_on(veth_setup::run_veth_link(
            &dev1_if_name_clone,
            &dev2_if_name_clone,
            startup_w,
            shutdown_r,
        ))
    });

    loop {
        match startup_r.try_recv() {
            Ok(_) => break,
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Closed) => panic!("Failed to set up veth link"),
        }
    }

    // Socket for the first interfaace
    let ((umem, fill_q, comp_q, frame_descs), (tx_q, rx_q)) = xsk_setup::build_socket_and_umem(
        umem_config.clone(),
        socket_config.clone(),
        &dev1_if_name,
        0,
    );

    let dev1_socket = SocketState {
        if_name: dev1_if_name,
        umem,
        fill_q,
        comp_q,
        tx_q,
        rx_q,
        frame_descs,
    };

    // Socket for the second interface
    let ((umem, fill_q, comp_q, frame_descs), (tx_q, rx_q)) =
        xsk_setup::build_socket_and_umem(umem_config, socket_config, &dev2_if_name, 0);

    let dev2_socket = SocketState {
        if_name: dev2_if_name,
        umem,
        fill_q,
        comp_q,
        tx_q,
        rx_q,
        frame_descs,
    };

    bench_fn(dev1_socket, dev2_socket);

    if let Err(e) = shutdown_w.send(()) {
        eprintln!("veth link thread returned unexpectedly: {:?}", e);
    }

    veth_handle.join().unwrap();
}
