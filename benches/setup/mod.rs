use rust_xsk::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
};

mod veth_setup;
mod xsk_setup;

pub use xsk_setup::{SocketConfigBuilder, UmemConfigBuilder};

pub struct SocketState {
    pub if_name: String,
    pub umem: Umem,
    pub fill_q: FillQueue,
    pub comp_q: CompQueue,
    pub socket: Socket,
    pub tx_q: TxQueue,
    pub rx_q: RxQueue,
}

pub async fn run_bench<F>(
    umem_config: Option<UmemConfig>,
    socket_config: Option<SocketConfig>,
    bench_fn: F,
    num_packets: u64,
) where
    F: Fn(u64, SocketState, SocketState) + Send + 'static,
{
    let inner = move |dev1_if_name: String, dev2_if_name: String| {
        // Create the socket for the first interfaace
        let ((umem, fill_q, comp_q), (socket, tx_q, rx_q)) = xsk_setup::build_socket_and_umem(
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
            socket,
            tx_q,
            rx_q,
        };

        let ((umem, fill_q, comp_q), (socket, tx_q, rx_q)) =
            xsk_setup::build_socket_and_umem(umem_config, socket_config, &dev2_if_name, 0);

        let dev2_socket = SocketState {
            if_name: dev2_if_name,
            umem,
            fill_q,
            comp_q,
            socket,
            tx_q,
            rx_q,
        };

        bench_fn(num_packets, dev1_socket, dev2_socket)
    };

    veth_setup::run_with_dev(inner).await;
}
