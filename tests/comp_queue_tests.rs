use serial_test::serial;
use std::{thread, time::Duration};
use xsk_rs::{socket::Config as SocketConfig, umem::Config as UmemConfig};

mod setup;
use setup::{SocketConfigBuilder, UmemConfigBuilder, Xsk};

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
#[serial]
async fn comp_queue_consumes_nothing_if_tx_q_unused() {
    fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
        assert!(dev1.comp_q.consume().is_empty());
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
async fn num_frames_consumed_match_those_produced() {
    fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
        let mut dev1_frames = dev1.frames;

        let mut frames_to_send = Vec::with_capacity(2);
        for _ in 0..2 {
            frames_to_send.push(dev1_frames.pop().expect("got enough frames"));
        }
        let not_sent = dev1.tx_q.produce_and_wakeup(frames_to_send).unwrap();
        assert!(not_sent.is_empty());

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        let mut frames_completed = dev1.comp_q.consume();
        assert_eq!(frames_completed.len(), 2);
        dev1_frames.append(&mut frames_completed);
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

// ToDo: Fix test
/*
#[tokio::test]
#[serial]
async fn addr_of_frames_consumed_match_addr_of_those_produced() {
    fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
        let dev1_tx_q_frames = dev1.frames;
        let mut dev1_comp_q_frames = dev1_tx_q_frames.clone();

        unsafe {
            dev1.tx_q
                .produce_and_wakeup(&dev1_tx_q_frames[2..4])
                .unwrap()
        };

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
*/
