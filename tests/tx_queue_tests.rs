use rust_xsk::{socket::*, umem::*};

mod setup;

use setup::{SocketConfigBuilder, UmemConfigBuilder};

#[tokio::test]
async fn tx_queue_produce_tx_size_frames() {
    fn test_fn(
        umem: Umem,
        _fill_q: FillQueue,
        _comp_q: CompQueue,
        _socket: Socket,
        mut tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(tx_q.produce(&frame_descs[..4]), 4);
    }

    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        ..UmemConfigBuilder::default()
    }
    .build();

    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    setup::run_test(Some(umem_config), Some(socket_config), test_fn).await;
}

#[tokio::test]
async fn tx_queue_produce_gt_tx_size_frames() {
    fn test_fn(
        umem: Umem,
        _fill_q: FillQueue,
        _comp_q: CompQueue,
        _socket: Socket,
        mut tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(tx_q.produce(&frame_descs[..5]), 4);
    }

    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        ..UmemConfigBuilder::default()
    }
    .build();

    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    setup::run_test(Some(umem_config), Some(socket_config), test_fn).await;
}

#[tokio::test]
async fn tx_queue_produce_frames_until_tx_queue_full() {
    fn test_fn(
        umem: Umem,
        _fill_q: FillQueue,
        _comp_q: CompQueue,
        _socket: Socket,
        mut tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(tx_q.produce(&frame_descs[..2]), 2);
        assert_eq!(tx_q.produce(&frame_descs[2..5]), 2);
        assert_eq!(tx_q.produce(&frame_descs[5..8]), 0);
    }

    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        ..UmemConfigBuilder::default()
    }
    .build();

    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    setup::run_test(Some(umem_config), Some(socket_config), test_fn).await;
}

#[tokio::test]
async fn tx_queue_produce_and_wakeup() {
    fn test_fn(
        umem: Umem,
        _fill_q: FillQueue,
        _comp_q: CompQueue,
        _socket: Socket,
        mut tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(tx_q.produce_and_wakeup(&frame_descs[..5]).unwrap(), 4);
    }

    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        ..UmemConfigBuilder::default()
    }
    .build();

    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    setup::run_test(Some(umem_config), Some(socket_config), test_fn).await;
}
