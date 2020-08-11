use rust_xsk::{socket::*, umem::*};

mod setup;

use setup::UmemConfigBuilder;

#[tokio::test]
async fn fill_queue_produce_tx_size_frames() {
    fn test_fn(
        umem: Umem,
        mut fill_q: FillQueue,
        _comp_q: CompQueue,
        _socket: Socket,
        _tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(fill_q.produce(&frame_descs[..4]), 4);
    }

    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        fill_queue_size: 4,
        ..UmemConfigBuilder::default()
    }
    .build();

    setup::run_test(Some(umem_config), None, test_fn).await;
}

#[tokio::test]
async fn fill_queue_produce_gt_tx_size_frames() {
    fn test_fn(
        umem: Umem,
        mut fill_q: FillQueue,
        _comp_q: CompQueue,
        _socket: Socket,
        _tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(fill_q.produce(&frame_descs[..5]), 4);
    }

    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        fill_queue_size: 4,
        ..UmemConfigBuilder::default()
    }
    .build();

    setup::run_test(Some(umem_config), None, test_fn).await;
}

#[tokio::test]
async fn fill_queue_produce_frames_until_full() {
    fn test_fn(
        umem: Umem,
        mut fill_q: FillQueue,
        _comp_q: CompQueue,
        _socket: Socket,
        _tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(fill_q.produce(&frame_descs[..2]), 2);
        assert_eq!(fill_q.produce(&frame_descs[2..5]), 2);
        assert_eq!(fill_q.produce(&frame_descs[5..8]), 0);
    }

    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        fill_queue_size: 4,
        ..UmemConfigBuilder::default()
    }
    .build();

    setup::run_test(Some(umem_config), None, test_fn).await;
}

#[tokio::test]
async fn fill_queue_produce_and_wakeup() {
    fn test_fn(
        umem: Umem,
        mut fill_q: FillQueue,
        _comp_q: CompQueue,
        socket: Socket,
        _tx_q: TxQueue,
        _rx_q: RxQueue,
    ) {
        let frame_descs = umem.frame_descs();

        assert_eq!(
            fill_q
                .produce_and_wakeup(&frame_descs[..5], socket.file_descriptor(), 100)
                .unwrap(),
            4
        );
    }

    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        fill_queue_size: 4,
        ..UmemConfigBuilder::default()
    }
    .build();

    setup::run_test(Some(umem_config), None, test_fn).await;
}
