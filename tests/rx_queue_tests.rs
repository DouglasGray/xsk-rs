#[allow(dead_code)]
mod setup;
use setup::{PacketGenerator, Xsk, XskConfig};

use libbpf_sys::XDP_PACKET_HEADROOM;
use serial_test::serial;
use std::{convert::TryInto, io::Write, thread, time::Duration};
use xsk_rs::config::{FrameSize, QueueSize, SocketConfig, UmemConfig};

const CQ_SIZE: u32 = 4;
const FQ_SIZE: u32 = 4;
const TX_Q_SIZE: u32 = 4;
const RX_Q_SIZE: u32 = 4;
const FRAME_SIZE: u32 = 2048;
const FRAME_COUNT: u32 = 8;
const FRAME_HEADROOM: u32 = 512;

fn build_configs() -> (UmemConfig, SocketConfig) {
    let umem_config = UmemConfig::builder()
        .comp_queue_size(QueueSize::new(CQ_SIZE).unwrap())
        .fill_queue_size(QueueSize::new(FQ_SIZE).unwrap())
        .frame_size(FrameSize::new(FRAME_SIZE).unwrap())
        .frame_headroom(FRAME_HEADROOM)
        .build()
        .unwrap();

    let socket_config = SocketConfig::builder()
        .tx_queue_size(QueueSize::new(TX_Q_SIZE).unwrap())
        .rx_queue_size(QueueSize::new(RX_Q_SIZE).unwrap())
        .build();

    (umem_config, socket_config)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn rx_queue_consumes_nothing_if_no_tx_and_fill_q_empty() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        unsafe {
            assert_eq!(xsk1.rx_q.consume(&mut xsk1.frames[..2]), 0);

            assert_eq!(
                xsk1.rx_q
                    .poll_and_consume(&mut xsk1.frames[..2], 100)
                    .unwrap(),
                0
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn rx_queue_consume_returns_nothing_if_fill_q_empty() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        unsafe {
            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.frames[..4]).unwrap(), 4);

            assert_eq!(xsk1.rx_q.consume(&mut xsk1.frames[..4]), 0);

            assert_eq!(
                xsk1.rx_q
                    .poll_and_consume(&mut xsk1.frames[..4], 100)
                    .unwrap(),
                0
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn rx_queue_consumes_frame_correctly_after_tx() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        unsafe {
            // Add a frame in the dev2 fill queue ready to receive
            assert_eq!(xsk2.fq.produce(&xsk2.frames[0..1]), 1);

            // Write to frame of dev 1
            let sent_pkt = b"hello";

            xsk1.frames[0]
                .data_mut()
                .cursor()
                .write_all(sent_pkt)
                .unwrap();

            assert_eq!(xsk1.frames[0].len(), 5);

            // Send data
            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.frames[..1]).unwrap(), 1);

            thread::sleep(Duration::from_millis(5));

            // Read on dev2
            assert_eq!(xsk2.rx_q.consume(&mut xsk2.frames), 1);

            assert_eq!(xsk2.frames[0].len(), 5);

            // Check that the data is correct
            assert_eq!(xsk2.frames[0].data().contents(), sent_pkt);
            assert_eq!(xsk2.frames[0].data_mut().contents(), sent_pkt);
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn recvd_packet_offset_after_tx_includes_xdp_and_frame_headroom() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        unsafe {
            let mut xsk1 = dev1.0;
            let mut xsk2 = dev2.0;

            // Add a frame in the dev2 fill queue ready to receive
            assert_eq!(xsk2.fq.produce(&xsk2.frames[0..1]), 1);

            // Data to send from dev1
            let sent_pkt = b"hello";

            // Write data to UMEM
            xsk1.frames[0]
                .data_mut()
                .cursor()
                .write_all(sent_pkt)
                .unwrap();

            assert_eq!(xsk1.frames[0].len(), 5);

            // Transmit data
            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.frames[..1]).unwrap(), 1);

            thread::sleep(Duration::from_millis(5));

            // Read on dev2
            assert_eq!(xsk2.rx_q.consume(&mut xsk2.frames), 1);

            assert_eq!(xsk2.frames[0].len(), 5);

            // Check that the data is correct
            assert_eq!(xsk2.frames[0].data().contents(), sent_pkt);
            assert_eq!(xsk2.frames[0].data_mut().contents(), sent_pkt);

            // Check addr starts where we expect
            assert_eq!(
                xsk2.frames[0].addr(),
                (XDP_PACKET_HEADROOM + FRAME_HEADROOM) as usize
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn headroom_data_present_after_receive() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        unsafe {
            let mut xsk1 = dev1.0;
            let mut xsk2 = dev2.0;

            // Write to dev2 frame headroom and put in fill queue
            xsk2.frames[0]
                .headroom_mut()
                .cursor()
                .write_all(b"hello")
                .unwrap();

            assert_eq!(xsk2.fq.produce(&xsk2.frames[0..1]), 1);

            // Send from dev1
            xsk1.frames[0]
                .data_mut()
                .cursor()
                .write_all(b"world")
                .unwrap();

            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.frames[..1]).unwrap(), 1);

            thread::sleep(Duration::from_millis(5));

            // Read on dev2
            assert_eq!(xsk2.rx_q.consume(&mut xsk2.frames), 1);

            assert_eq!(xsk2.frames[0].len(), 5);

            // Confirm headroom data still present after rx
            assert_eq!(xsk2.frames[0].headroom().contents(), b"hello");
            assert_eq!(xsk2.frames[0].headroom_mut().contents(), b"hello");

            // Check that the data is correct as well
            assert_eq!(xsk2.frames[0].data().contents(), b"world");
            assert_eq!(xsk2.frames[0].data_mut().contents(), b"world");
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn xdp_statistics_report_dropped_packet() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        unsafe {
            let mut xsk1 = dev1.0;
            let mut xsk2 = dev2.0;

            // Don't add frames to dev2's fill queue, just send from
            // dev1
            xsk1.frames[0]
                .data_mut()
                .cursor()
                .write_all(b"hello")
                .unwrap();

            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.frames[..1]).unwrap(), 1);

            // Try read - no frames in fill queue so should be zero
            assert_eq!(
                xsk2.rx_q.poll_and_consume(&mut xsk2.frames, 100).unwrap(),
                0
            );

            let stats = xsk2.rx_q.statistics().unwrap();

            assert!(stats.rx_dropped() > 0);
        }
    }

    build_configs_and_run_test(test).await
}

async fn build_configs_and_run_test<F>(test: F)
where
    F: Fn((Xsk, PacketGenerator), (Xsk, PacketGenerator)) + Send + 'static,
{
    let (dev1_umem_config, dev1_socket_config) = build_configs();
    let (dev2_umem_config, dev2_socket_config) = build_configs();

    setup::run_test(
        XskConfig {
            frame_count: FRAME_COUNT.try_into().unwrap(),
            umem_config: dev1_umem_config,
            socket_config: dev1_socket_config,
        },
        XskConfig {
            frame_count: FRAME_COUNT.try_into().unwrap(),
            umem_config: dev2_umem_config,
            socket_config: dev2_socket_config,
        },
        test,
    )
    .await;
}
