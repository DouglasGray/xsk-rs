#[allow(dead_code)]
mod setup;
use setup::{PacketGenerator, Xsk, XskConfig, ETHERNET_PACKET};

use serial_test::serial;
use std::{convert::TryInto, io::Write, thread, time::Duration};
use xsk_rs::config::{QueueSize, SocketConfig, UmemConfig};
use xsk_rs::umem::frame::FrameDesc;

const CQ_SIZE: u32 = 16;
const TX_Q_SIZE: u32 = 16;
const FRAME_COUNT: u32 = 32;

fn build_configs() -> (UmemConfig, SocketConfig) {
    let umem_config = UmemConfig::builder()
        .comp_queue_size(QueueSize::new(CQ_SIZE).unwrap())
        .build()
        .unwrap();

    let socket_config = SocketConfig::builder()
        .tx_queue_size(QueueSize::new(TX_Q_SIZE).unwrap())
        .build();

    (umem_config, socket_config)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn comp_queue_consumes_nothing_if_tx_q_unused() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        unsafe {
            assert_eq!(xsk1.cq.consume(&mut xsk1.descs), 0);
        }

        unsafe {
            assert_eq!(xsk1.cq.consume_one(&mut xsk1.descs[0]), 0);
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn num_frames_consumed_match_those_produced() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        for i in 0..2 {
            unsafe {
                xsk1.umem
                    .data_mut(&mut xsk1.descs[i])
                    .cursor()
                    .write_all(&ETHERNET_PACKET[..])
                    .unwrap();
            }
        }

        assert_eq!(
            unsafe { xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..2]).unwrap() },
            2
        );

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        assert_eq!(unsafe { xsk1.cq.consume(&mut xsk1.descs) }, 2);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn consume_one_should_consume_a_single_frame_even_if_multiple_produced() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        for i in 0..2 {
            unsafe {
                xsk1.umem
                    .data_mut(&mut xsk1.descs[i])
                    .cursor()
                    .write_all(&ETHERNET_PACKET[..])
                    .unwrap();
            }
        }
        assert_eq!(
            unsafe { xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..2]).unwrap() },
            2
        );

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        assert_eq!(unsafe { xsk1.cq.consume_one(&mut xsk1.descs[0]) }, 1);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn addr_of_frames_consumed_match_addr_of_those_produced() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let nb = (FRAME_COUNT / 2) as usize;

        assert!(nb > 0);

        let (tx_frames, rx_frames) = xsk1.descs.split_at_mut(nb);

        for i in 0..nb {
            unsafe {
                xsk1.umem
                    .data_mut(&mut tx_frames[i])
                    .cursor()
                    .write_all(&ETHERNET_PACKET[..])
                    .unwrap();
            }
        }
        assert_eq!(
            unsafe { xsk1.tx_q.produce_and_wakeup(&tx_frames).unwrap() },
            nb
        );

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        assert_eq!(unsafe { xsk1.cq.consume(&mut rx_frames[..nb]) }, nb);

        let mut txd_addrs = tx_frames
            .iter()
            .map(FrameDesc::addr)
            .collect::<Vec<usize>>();

        let mut rxd_addrs = rx_frames[..nb]
            .iter()
            .map(FrameDesc::addr)
            .collect::<Vec<usize>>();

        txd_addrs.sort();
        rxd_addrs.sort();

        assert_eq!(txd_addrs, rxd_addrs);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn frame_consumed_with_consume_one_should_match_addr_of_one_produced() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let nb = (FRAME_COUNT / 2) as usize;

        assert!(nb > 0);

        let (tx_frames, rx_frames) = xsk1.descs.split_at_mut(nb);

        unsafe {
            xsk1.umem
                .data_mut(&mut tx_frames[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();
        }

        assert_eq!(
            unsafe { xsk1.tx_q.produce_and_wakeup(&tx_frames).unwrap() },
            nb
        );

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        assert_eq!(unsafe { xsk1.cq.consume_one(&mut rx_frames[0]) }, 1);

        assert!(tx_frames.iter().any(|f| rx_frames[0].addr() == f.addr()));
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
