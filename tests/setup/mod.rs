use rust_xsk::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
};

mod veth_setup;
mod xsk_setup;

pub use xsk_setup::{SocketConfigBuilder, UmemConfigBuilder};

pub struct SocketState<'a> {
    pub if_name: String,
    pub umem: Umem<'a>,
    pub fill_q: FillQueue<'a>,
    pub comp_q: CompQueue<'a>,
    pub tx_q: TxQueue<'a>,
    pub rx_q: RxQueue<'a>,
    pub frame_descs: Vec<FrameDesc>,
}

pub async fn run_test<F>(
    umem_config: Option<UmemConfig>,
    socket_config: Option<SocketConfig>,
    test: F,
) where
    F: Fn(SocketState, SocketState) + Send + 'static,
{
    let inner = move |dev1_if_name: String, dev2_if_name: String| {
        // Create the socket for the first interfaace
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

        test(dev1_socket, dev2_socket)
    };

    veth_setup::run_with_dev(inner).await;
}
