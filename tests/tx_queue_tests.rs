use rust_xsk::{
    socket::{Config as SocketConfig, *},
    umem::{Config as UmemConfig, *},
};

mod setup;

use setup::SocketConfigBuilder;

#[tokio::test]
async fn tx_queue_produce_lt_tx_size_frames() {
    fn test_fn(
        umem: Umem,
        _fill_q: FillQueue,
        _comp_q: CompQueue,
        _socket: Socket,
        mut tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(tx_q.produce(&frame_descs[..3]), 3);
    }

    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    setup::run_test(None, Some(socket_config), test_fn).await;
}
