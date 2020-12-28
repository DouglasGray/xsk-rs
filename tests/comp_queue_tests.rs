use std::{thread, time::Duration};

use xsk_rs::{socket::Config as SocketConfig, umem::Config as UmemConfig};

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        comp_queue_size: 4,
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
async fn comp_queue_consumes_nothing_if_tx_q_unused() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let mut dev1_frames = dev1.frame_descs;

        assert_eq!(dev1.comp_q.consume(&mut dev1_frames[..4]), 0);
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
async fn num_frames_consumed_match_those_produced() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let mut dev1_frames = dev1.frame_descs;

        assert_eq!(dev1.tx_q.produce_and_wakeup(&dev1_frames[..2]).unwrap(), 2);

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        assert_eq!(dev1.comp_q.consume(&mut dev1_frames[..4]), 2);
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
async fn addr_of_frames_consumed_match_addr_of_those_produced() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let dev1_tx_q_frames = dev1.frame_descs;
        let mut dev1_comp_q_frames = dev1_tx_q_frames.clone();

        dev1.tx_q
            .produce_and_wakeup(&dev1_tx_q_frames[2..4])
            .unwrap();

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        dev1.comp_q.consume(&mut dev1_comp_q_frames[..2]);

        // Also ensure that the frame info matches
        assert_eq!(
            &dev1_tx_q_frames[2..4]
                .iter()
                .map(|f| f.addr())
                .collect::<Vec<usize>>(),
            &dev1_comp_q_frames[..2]
                .iter()
                .map(|f| f.addr())
                .collect::<Vec<usize>>(),
        );
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
