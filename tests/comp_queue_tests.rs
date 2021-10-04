#[allow(dead_code)]
mod setup;
use setup::{PacketGenerator, Xsk, XskConfig};

use serial_test::serial;
use std::{convert::TryInto, thread, time::Duration};
use xsk_rs::{
    config::{QueueSize, SocketConfig, UmemConfig},
    umem::frame::Frame,
};

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
            assert_eq!(xsk1.cq.consume(&mut xsk1.frames), 0);
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn num_frames_consumed_match_those_produced() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        assert_eq!(
            unsafe { xsk1.tx_q.produce_and_wakeup(&xsk1.frames[..2]).unwrap() },
            2
        );

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        assert_eq!(unsafe { xsk1.cq.consume(&mut xsk1.frames) }, 2);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn addr_of_frames_consumed_match_addr_of_those_produced() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let nb = (FRAME_COUNT / 2) as usize;

        let (tx_frames, rx_frames) = xsk1.frames.split_at_mut(nb);

        unsafe { xsk1.tx_q.produce_and_wakeup(&tx_frames[..nb]).unwrap() };

        // Wait briefly so we don't try to consume too early
        thread::sleep(Duration::from_millis(5));

        unsafe { xsk1.cq.consume(&mut rx_frames[..nb]) };

        // Also ensure that the frame info matches
        assert_eq!(
            &tx_frames[..nb]
                .iter()
                .map(Frame::addr)
                .collect::<Vec<usize>>(),
            &rx_frames[..nb]
                .iter()
                .map(Frame::addr)
                .collect::<Vec<usize>>(),
        );
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
