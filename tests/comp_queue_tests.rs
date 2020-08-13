use std::{thread, time::Duration};

use rust_xsk::{socket::Config as SocketConfig, umem::Config as UmemConfig};

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
        let mut d1_comp_q_frames = dev1.umem.frame_descs().to_vec();

        assert_eq!(dev1.comp_q.consume(&mut d1_comp_q_frames[..4]), 0);
    }

    let (umem_config, socket_config) = build_configs();

    setup::run_test(umem_config, socket_config, test_fn).await;
}

#[tokio::test]
async fn num_frames_consumed_match_those_produced() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let d1_tx_q_frames = dev1.umem.frame_descs().to_vec();
        let mut d1_comp_q_frames = dev1.umem.frame_descs().to_vec();

        assert_eq!(
            dev1.tx_q.produce_and_wakeup(&d1_tx_q_frames[..2]).unwrap(),
            2
        );

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        assert_eq!(dev1.comp_q.consume(&mut d1_comp_q_frames[..4]), 2);
    }

    let (umem_config, socket_config) = build_configs();

    setup::run_test(umem_config, socket_config, test_fn).await;
}

#[tokio::test]
async fn addr_of_frames_consumed_match_addr_of_those_produced() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let d1_tx_q_frames = dev1.umem.frame_descs().to_vec();
        let mut d1_comp_q_frames = dev1.umem.frame_descs().to_vec();

        dev1.tx_q.produce_and_wakeup(&d1_tx_q_frames[2..4]).unwrap();

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        dev1.comp_q.consume(&mut d1_comp_q_frames[..2]);

        // Also ensure that the frame info matches
        assert_eq!(
            &d1_tx_q_frames[2..4]
                .iter()
                .map(|f| f.addr())
                .collect::<Vec<u64>>(),
            &d1_comp_q_frames[..2]
                .iter()
                .map(|f| f.addr())
                .collect::<Vec<u64>>(),
        );
    }

    let (umem_config, socket_config) = build_configs();

    setup::run_test(umem_config, socket_config, test_fn).await;
}
