use xsk_rs::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
};

mod veth_setup;

mod xsk_setup;
use std::sync::Arc;
pub use xsk_setup::{SocketConfigBuilder, UmemConfigBuilder};

pub struct Xsk<'a> {
    pub if_name: String,
    pub fill_q: FillQueue<'a>,
    pub comp_q: CompQueue<'a>,
    pub tx_q: TxQueue<'a>,
    pub rx_q: RxQueue<'a>,
    pub frames: Vec<Frame>,
    pub umem: Arc<Umem<'a>>,
}

pub async fn run_test<F>(
    dev1_umem_config: Option<UmemConfig>,
    dev1_socket_config: Option<SocketConfig>,
    dev2_umem_config: Option<UmemConfig>,
    dev2_socket_config: Option<SocketConfig>,
    test: F,
) where
    F: Fn(Xsk, Xsk) + Send + 'static,
{
    let inner = move |dev1_if_name: String, dev2_if_name: String| {
        // Create the socket for the first interfaace
        let ((umem, fill_q, comp_q, frame_descs), (tx_q, rx_q)) = xsk_setup::build_socket_and_umem(
            dev1_umem_config,
            dev1_socket_config,
            &dev1_if_name,
            0,
        );

        let dev1_socket = Xsk {
            if_name: dev1_if_name,
            fill_q,
            comp_q,
            tx_q,
            rx_q,
            frames: frame_descs,
            umem,
        };

        let ((umem, fill_q, comp_q, frame_descs), (tx_q, rx_q)) = xsk_setup::build_socket_and_umem(
            dev2_umem_config,
            dev2_socket_config,
            &dev2_if_name,
            0,
        );

        let dev2_socket = Xsk {
            if_name: dev2_if_name,
            fill_q,
            comp_q,
            tx_q,
            rx_q,
            frames: frame_descs,
            umem,
        };

        test(dev1_socket, dev2_socket)
    };

    veth_setup::run_with_dev(inner).await.unwrap();
}
