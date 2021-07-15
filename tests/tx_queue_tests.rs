use serial_test::serial;
use xsk_rs::{socket::Config as SocketConfig, umem::Config as UmemConfig};

mod setup;
use setup::{SocketConfigBuilder, UmemConfigBuilder, Xsk};

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
#[serial]
async fn tx_queue_produce_tx_size_frames() {
    fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
        let frame_descs = dev1.frame_descs;

        assert_eq!(unsafe { dev1.tx_q.produce(&frame_descs[..4]) }, 4);
    }

    let (dev1_umem_config, dev1_socket_config) = build_configs();
    let (dev2_umem_config, dev2_socket_config) = build_configs();

    setup::run_test(
        dev1_umem_config,
        dev1_socket_config,
        dev2_umem_config,
        dev2_socket_config,
        test_fn,
    )
    .await;
}

#[tokio::test]
#[serial]
async fn tx_queue_produce_gt_tx_size_frames() {
    fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
        let frame_descs = dev1.frame_descs;

        assert_eq!(unsafe { dev1.tx_q.produce(&frame_descs[..5]) }, 0);
    }

    let (dev1_umem_config, dev1_socket_config) = build_configs();
    let (dev2_umem_config, dev2_socket_config) = build_configs();

    setup::run_test(
        dev1_umem_config,
        dev1_socket_config,
        dev2_umem_config,
        dev2_socket_config,
        test_fn,
    )
    .await;
}

#[tokio::test]
#[serial]
async fn tx_queue_produce_frames_until_tx_queue_full() {
    fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
        let frame_descs = dev1.frame_descs;

        assert_eq!(unsafe { dev1.tx_q.produce(&frame_descs[..2]) }, 2);
        assert_eq!(unsafe { dev1.tx_q.produce(&frame_descs[2..3]) }, 1);
        assert_eq!(unsafe { dev1.tx_q.produce(&frame_descs[3..8]) }, 0);
        assert_eq!(unsafe { dev1.tx_q.produce(&frame_descs[3..4]) }, 1);
    }

    let (dev1_umem_config, dev1_socket_config) = build_configs();
    let (dev2_umem_config, dev2_socket_config) = build_configs();

    setup::run_test(
        dev1_umem_config,
        dev1_socket_config,
        dev2_umem_config,
        dev2_socket_config,
        test_fn,
    )
    .await;
}
