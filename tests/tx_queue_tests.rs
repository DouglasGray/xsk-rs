use rust_xsk::{socket::Config as SocketConfig, umem::Config as UmemConfig};

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
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

    (Some(umem_config), Some(socket_config))
}

#[tokio::test]
async fn tx_queue_produce_tx_size_frames() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let frame_descs = dev1.umem.frame_descs();

        assert_eq!(dev1.tx_q.produce(&frame_descs[..4]), 4);
    }

    let (umem_config, socket_config) = build_configs();

    setup::run_test(umem_config, socket_config, test_fn).await;
}

#[tokio::test]
async fn tx_queue_produce_gt_tx_size_frames() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let frame_descs = dev1.umem.frame_descs();

        assert_eq!(dev1.tx_q.produce(&frame_descs[..5]), 4);
    }

    let (umem_config, socket_config) = build_configs();

    setup::run_test(umem_config, socket_config, test_fn).await;
}

#[tokio::test]
async fn tx_queue_produce_frames_until_tx_queue_full() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let frame_descs = dev1.umem.frame_descs();

        assert_eq!(dev1.tx_q.produce(&frame_descs[..2]), 2);
        assert_eq!(dev1.tx_q.produce(&frame_descs[2..5]), 2);
        assert_eq!(dev1.tx_q.produce(&frame_descs[5..8]), 0);
    }

    let (umem_config, socket_config) = build_configs();

    setup::run_test(umem_config, socket_config, test_fn).await;
}
