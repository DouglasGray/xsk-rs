use libbpf_sys::XDP_PACKET_HEADROOM;
use serial_test::serial;
use xsk_rs::{socket::Config as SocketConfig, umem::Config as UmemConfig};

mod setup;
use setup::{SocketConfigBuilder, UmemConfigBuilder, Xsk};

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

fn default_config_builders() -> (UmemConfigBuilder, SocketConfigBuilder) {
    let umem_config_builder = UmemConfigBuilder {
        frame_count: 8,
        frame_size: 2048,
        fill_queue_size: 4,
        comp_queue_size: 4,
        ..UmemConfigBuilder::default()
    };

    let socket_config_builder = SocketConfigBuilder {
        tx_queue_size: 4,
        rx_queue_size: 4,
        ..SocketConfigBuilder::default()
    };

    (umem_config_builder, socket_config_builder)
}

#[tokio::test]
#[serial]
async fn rx_queue_consumes_nothing_if_no_tx_and_fill_q_empty() {
    fn test_fn(mut dev1: Xsk, _dev2: Xsk) {
        assert_eq!(dev1.rx_q.consume().len(), 0);
        assert_eq!(dev1.rx_q.poll_and_consume(100).unwrap().len(), 0);
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
async fn rx_queue_consume_returns_nothing_if_fill_q_empty() {
    fn test_fn(mut dev1: Xsk, mut dev2: Xsk) {
        let mut frames_to_send = Vec::with_capacity(4);
        for _ in 0..4 {
            frames_to_send.push(dev2.frames.pop().unwrap());
        }

        let not_sent = dev2.tx_q.produce_and_wakeup(frames_to_send).unwrap();
        assert!(not_sent.is_empty());

        assert!(dev1.rx_q.consume().is_empty());
        assert!(dev1.rx_q.poll_and_consume(100).unwrap().is_empty());
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

// ToDo: Fix tests
/*
#[tokio::test]
#[serial]
async fn rx_queue_consumes_frame_correctly_after_tx() {
    fn test_fn(mut dev1: Xsk, mut dev2: Xsk) {
        // Add a frame in the dev1 fill queue ready to receive
        assert_eq!(unsafe { dev1.fill_q.produce(&dev1.frames[0..1]) }, 1);

        // Data to send from dev2
        let pkt = vec![b'H', b'e', b'l', b'l', b'o'];

        // Write data to UMEM
        unsafe {
            dev2.umem
                .write_to_umem_checked(&mut dev2.frames[0], &pkt[..])
                .unwrap();
        }

        assert_eq!(dev2.frames[0].len(), 5);

        // Transmit data
        assert_eq!(
            unsafe { dev2.tx_q.produce_and_wakeup(&dev2.frames[0..1]).unwrap() },
            1
        );

        // Read on dev1
        assert_eq!(dev1.rx_q.consume(&mut dev1.frames[..]), 1);

        assert_eq!(dev1.frames[0].len(), 5);

        // Check that the data is correct
        let recvd = unsafe {
            dev1.umem
                .read_from_umem_checked(&dev1.frames[0].addr(), &dev1.frames[0].len())
                .unwrap()
        };

        assert_eq!(recvd[..5], pkt[..]);
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
async fn recvd_packet_offset_after_tx_includes_xdp_and_frame_headroom() {
    fn test_fn(mut dev1: Xsk, mut dev2: Xsk) {
        // Add a frame in the dev1 fill queue ready to receive
        assert_eq!(unsafe { dev1.fill_q.produce(&dev1.frames[0..1]) }, 1);

        // Data to send from dev2
        let pkt = vec![b'H', b'e', b'l', b'l', b'o'];

        // Write data to UMEM
        unsafe {
            dev2.umem
                .write_to_umem_checked(&mut dev2.frames[0], &pkt[..])
                .unwrap();
        }

        assert_eq!(dev2.frames[0].len(), 5);

        // Transmit data
        assert_eq!(
            unsafe { dev2.tx_q.produce_and_wakeup(&dev2.frames[0..1]).unwrap() },
            1
        );

        // Read on dev1
        assert_eq!(dev1.rx_q.consume(&mut dev1.frames[..]), 1);

        assert_eq!(dev1.frames[0].len(), 5);

        // Check addr starts where we expect
        assert_eq!(dev1.frames[0].addr(), (XDP_PACKET_HEADROOM + 512) as usize);
    }

    let (dev1_umem_config_builder, dev1_socket_config_builder) = default_config_builders();
    let (dev2_umem_config, dev2_socket_config) = build_configs();

    // Add to the frame headroom
    let dev1_umem_config_builder = UmemConfigBuilder {
        frame_headroom: 512,
        ..dev1_umem_config_builder
    };

    let dev1_umem_config = Some(dev1_umem_config_builder.build());
    let dev1_socket_config = Some(dev1_socket_config_builder.build());

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
