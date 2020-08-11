use rust_xsk::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
};

mod veth_setup;
mod xsk_setup;

pub use xsk_setup::{SocketConfigBuilder, UmemConfigBuilder};

pub async fn run_test<F>(
    umem_config: Option<UmemConfig>,
    socket_config: Option<SocketConfig>,
    test: F,
) where
    F: Fn(Umem, FillQueue, CompQueue, Socket, TxQueue, RxQueue) + Send + 'static,
{
    let inner = move |if_name: String| {
        let ((umem, fill_q, comp_q), (socket, tx_q, rx_q)) =
            xsk_setup::build_socket_and_umem(umem_config, socket_config, &if_name, 0);

        test(umem, fill_q, comp_q, socket, tx_q, rx_q)
    };

    veth_setup::with_dev(inner).await;
}
