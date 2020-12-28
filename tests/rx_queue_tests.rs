use xsk_rs::{socket::Config as SocketConfig, umem::Config as UmemConfig};

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfigBuilder {
        frame_count: 8,
        frame_size: 2048,
        fill_queue_size: 4,
        comp_queue_size: 4,
        ..UmemConfigBuilder::default()
    }
    .build();

    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        rx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    (Some(umem_config), Some(socket_config))
}

#[tokio::test]
async fn rx_queue_consumes_nothing_if_no_tx_and_fill_q_empty() {
    fn test_fn(mut dev1: SocketState, _dev2: SocketState) {
        let mut d1_rx_q_frames = dev1.frame_descs;

        assert_eq!(dev1.rx_q.consume(&mut d1_rx_q_frames[..2]), 0);
        assert_eq!(
            dev1.rx_q
                .poll_and_consume(&mut d1_rx_q_frames[..2], 100)
                .unwrap(),
            0
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

#[tokio::test]
async fn rx_queue_consume_returns_nothing_if_fill_q_empty() {
    fn test_fn(mut dev1: SocketState, mut dev2: SocketState) {
        let mut d1_rx_q_frames = dev1.frame_descs;
        let d2_tx_q_frames = dev2.frame_descs;

        assert_eq!(
            dev2.tx_q.produce_and_wakeup(&d2_tx_q_frames[..4]).unwrap(),
            4
        );

        assert_eq!(dev1.rx_q.consume(&mut d1_rx_q_frames[..4]), 0);
        assert_eq!(
            dev1.rx_q
                .poll_and_consume(&mut d1_rx_q_frames[..4], 100)
                .unwrap(),
            0
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

#[tokio::test]
async fn rx_queue_consumes_frame_correctly_after_tx() {
    fn test_fn(mut dev1: SocketState, mut dev2: SocketState) {
        let d1_fill_q_frames = dev1.frame_descs;
        let mut d1_rx_q_frames = d1_fill_q_frames.clone();

        let mut d2_tx_q_frames = dev2.frame_descs;

        // Add a frame in the fill queue ready for the rx path to fill in
        assert_eq!(dev1.fill_q.produce(&d1_fill_q_frames[0..1]), 1);

        // Pretend we're sending some data from dev2
        let pkt = vec![b'H', b'e', b'l', b'l', b'o'];

        assert_eq!(d2_tx_q_frames[0].len(), 0);

        unsafe {
            dev2.umem
                .copy_data_to_frame(&mut d2_tx_q_frames[0], &pkt[..])
                .unwrap();
        }

        assert_eq!(d2_tx_q_frames[0].len(), 5);

        // Send the frame
        assert_eq!(
            dev2.tx_q.produce_and_wakeup(&d2_tx_q_frames[0..1]).unwrap(),
            1
        );

        // Now read on dev1
        assert_eq!(dev1.rx_q.consume(&mut d1_rx_q_frames[..]), 1);
        assert_eq!(d1_rx_q_frames[0].len(), 5);

        // Check that the frame data is correct
        let frame_ref = unsafe {
            dev1.umem
                .frame_ref_at_addr(&d1_rx_q_frames[0].addr())
                .unwrap()
        };

        assert_eq!(frame_ref[..5], pkt[..]);
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
