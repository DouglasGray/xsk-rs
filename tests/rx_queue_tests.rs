#[allow(dead_code)]
mod setup;
use setup::{ETHERNET_PACKET, PacketGenerator, WAIT_TIMEOUT, Xsk, XskConfig, wait_until};

use libxdp_sys::XDP_PACKET_HEADROOM;
use serial_test::serial;
use std::{convert::TryInto, io::Write, thread, time::Duration};
use xsk_rs::config::{FrameSize, QueueSize, SocketConfig, UmemConfig, XDP_UMEM_MIN_CHUNK_SIZE};

const CQ_SIZE: u32 = 4;
const FQ_SIZE: u32 = 4;
const TX_Q_SIZE: u32 = 4;
const RX_Q_SIZE: u32 = 4;
const FRAME_SIZE: u32 = XDP_UMEM_MIN_CHUNK_SIZE;
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
async fn nothing_is_consumed_when_no_tx_sent_and_fill_q_empty() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        unsafe {
            assert_eq!(xsk1.rx_q.consume(&mut xsk1.descs[..2]), 0);

            assert_eq!(
                xsk1.rx_q
                    .poll_and_consume(&mut xsk1.descs[..2], 100)
                    .unwrap(),
                0
            );

            assert_eq!(xsk1.rx_q.consume_one(&mut xsk1.descs[0]), 0);

            assert_eq!(
                xsk1.rx_q
                    .poll_and_consume_one(&mut xsk1.descs[0], 100)
                    .unwrap(),
                0
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nothing_is_consumed_when_tx_sent_but_fill_q_empty() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        unsafe {
            assert_eq!(xsk2.tx_q.produce_and_wakeup(&xsk2.descs[..4]).unwrap(), 4);

            assert_eq!(xsk1.rx_q.consume(&mut xsk1.descs[..4]), 0);

            assert_eq!(
                xsk1.rx_q
                    .poll_and_consume(&mut xsk1.descs[..4], 100)
                    .unwrap(),
                0
            );

            assert_eq!(xsk1.rx_q.consume_one(&mut xsk1.descs[0]), 0);

            assert_eq!(
                xsk1.rx_q
                    .poll_and_consume_one(&mut xsk1.descs[0], 100)
                    .unwrap(),
                0
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn consumed_frame_data_matches_what_was_sent() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        unsafe {
            // Add a frame in the dev2 fill queue ready to receive
            assert_eq!(xsk2.fq.produce(&xsk2.descs[0..1]), 1);

            xsk1.umem
                .data_mut(&mut xsk1.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk1.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);

            // Send data
            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..1]).unwrap(), 1);

            // Read on dev2
            assert_eq!(xsk2.rx_q.poll_and_consume(&mut xsk2.descs, 100).unwrap(), 1);

            assert_eq!(xsk2.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);

            // Check that the data is correct
            assert_eq!(xsk2.umem.data(&xsk2.descs[0]).contents(), ETHERNET_PACKET);
            assert_eq!(
                xsk2.umem.data_mut(&mut xsk2.descs[0]).contents(),
                ETHERNET_PACKET
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn consume_one_frame_data_matches_what_was_sent() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        unsafe {
            // Add a frame in the dev2 fill queue ready to receive
            assert_eq!(xsk2.fq.produce(&xsk2.descs[0..1]), 1);

            xsk1.umem
                .data_mut(&mut xsk1.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk1.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);

            // Send data
            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..1]).unwrap(), 1);

            // Read on dev2
            assert_eq!(
                xsk2.rx_q
                    .poll_and_consume_one(&mut xsk2.descs[0], 100)
                    .unwrap(),
                1
            );

            assert_eq!(xsk2.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);

            // Check that the data is correct
            assert_eq!(xsk2.umem.data(&xsk2.descs[0]).contents(), ETHERNET_PACKET);
            assert_eq!(
                xsk2.umem.data_mut(&mut xsk2.descs[0]).contents(),
                ETHERNET_PACKET
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn consumed_frame_addresses_include_xdp_and_frame_headroom() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        unsafe {
            let mut xsk1 = dev1.0;
            let mut xsk2 = dev2.0;

            // Add a frame in the dev2 fill queue ready to receive
            assert_eq!(xsk2.fq.produce(&xsk2.descs[0..1]), 1);

            xsk1.umem
                .data_mut(&mut xsk1.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk1.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);

            // Transmit data
            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..1]).unwrap(), 1);

            // Read on dev2
            assert_eq!(xsk2.rx_q.poll_and_consume(&mut xsk2.descs, 100).unwrap(), 1);

            assert_eq!(xsk2.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);

            // Check that the data is correct
            assert_eq!(xsk2.umem.data(&xsk2.descs[0]).contents(), ETHERNET_PACKET);
            assert_eq!(
                xsk2.umem.data_mut(&mut xsk2.descs[0]).contents(),
                ETHERNET_PACKET
            );

            // Check addr starts where we expect
            assert_eq!(
                xsk2.descs[0].addr(),
                (XDP_PACKET_HEADROOM + FRAME_HEADROOM) as u64
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn consume_one_frame_address_includes_xdp_and_frame_headroom() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        unsafe {
            let mut xsk1 = dev1.0;
            let mut xsk2 = dev2.0;

            // Add a frame in the dev2 fill queue ready to receive
            assert_eq!(xsk2.fq.produce(&xsk2.descs[0..1]), 1);

            xsk1.umem
                .data_mut(&mut xsk1.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk1.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);

            // Transmit data
            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..1]).unwrap(), 1);

            // Read on dev2
            assert_eq!(
                xsk2.rx_q
                    .poll_and_consume_one(&mut xsk2.descs[0], 100)
                    .unwrap(),
                1
            );

            assert_eq!(xsk2.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);

            // Check that the data is correct
            assert_eq!(xsk2.umem.data(&xsk2.descs[0]).contents(), ETHERNET_PACKET);
            assert_eq!(
                xsk2.umem.data_mut(&mut xsk2.descs[0]).contents(),
                ETHERNET_PACKET
            );

            // Check addr starts where we expect
            assert_eq!(
                xsk2.descs[0].addr(),
                (XDP_PACKET_HEADROOM + FRAME_HEADROOM) as u64
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn headroom_len_reset_after_receive() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        unsafe {
            let mut xsk1 = dev1.0;
            let mut xsk2 = dev2.0;

            // Write to dev2 frame headroom and put in fill queue
            xsk2.umem
                .headroom_mut(&mut xsk2.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk2.descs[0].lengths().data(), 0);
            assert_eq!(
                xsk2.descs[0].lengths().headroom(),
                ETHERNET_PACKET.len() as u32
            );

            assert_eq!(xsk2.fq.produce(&xsk2.descs[0..1]), 1);

            // Send from dev1
            xsk1.umem
                .data_mut(&mut xsk1.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..1]).unwrap(), 1);

            // Read on dev2
            assert_eq!(xsk2.rx_q.poll_and_consume(&mut xsk2.descs, 100).unwrap(), 1);

            assert_eq!(xsk2.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);
            assert_eq!(xsk2.descs[0].lengths().headroom(), 0);

            // Length reset to zero but data should still be there
            xsk2.umem
                .headroom_mut(&mut xsk2.descs[0])
                .cursor()
                .set_pos(ETHERNET_PACKET.len() as u32);

            assert_eq!(
                xsk2.umem.headroom(&xsk2.descs[0]).contents(),
                &ETHERNET_PACKET[..]
            );
            assert_eq!(
                xsk2.umem.headroom_mut(&mut xsk2.descs[0]).contents(),
                &ETHERNET_PACKET[..]
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn consume_one_headroom_len_reset_after_receive() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        unsafe {
            let mut xsk1 = dev1.0;
            let mut xsk2 = dev2.0;

            // Write to dev2 frame headroom and put in fill queue
            xsk2.umem
                .headroom_mut(&mut xsk2.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk2.descs[0].lengths().data(), 0);
            assert_eq!(
                xsk2.descs[0].lengths().headroom(),
                ETHERNET_PACKET.len() as u32
            );

            assert_eq!(xsk2.fq.produce(&xsk2.descs[0..1]), 1);

            // Send from dev1
            xsk1.umem
                .data_mut(&mut xsk1.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..1]).unwrap(), 1);

            // Read on dev2
            assert_eq!(
                xsk2.rx_q
                    .poll_and_consume_one(&mut xsk2.descs[0], 100)
                    .unwrap(),
                1
            );

            assert_eq!(xsk2.descs[0].lengths().data(), ETHERNET_PACKET.len() as u32);
            assert_eq!(xsk2.descs[0].lengths().headroom(), 0);

            // Length reset to zero but data should still be there
            xsk2.umem
                .headroom_mut(&mut xsk2.descs[0])
                .cursor()
                .set_pos(ETHERNET_PACKET.len() as u32);

            assert_eq!(
                xsk2.umem.headroom(&xsk2.descs[0]).contents(),
                &ETHERNET_PACKET[..]
            );
            assert_eq!(
                xsk2.umem.headroom_mut(&mut xsk2.descs[0]).contents(),
                &ETHERNET_PACKET[..]
            );
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
            xsk1.umem
                .data_mut(&mut xsk1.descs[0])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();

            assert_eq!(xsk1.tx_q.produce_and_wakeup(&xsk1.descs[..1]).unwrap(), 1);

            // Try read - no frames in fill queue so should be zero
            assert_eq!(xsk2.rx_q.poll_and_consume(&mut xsk2.descs, 100).unwrap(), 0);

            let stats = xsk2.rx_q.fd().xdp_statistics().unwrap();

            assert!(stats.rx_dropped() > 0);
        }
    }

    build_configs_and_run_test(test).await
}

/// Writes a packet into each of the first `nb` frames of `xsk` and
/// sends them.
fn send(xsk: &mut Xsk, nb: usize) {
    for i in 0..nb {
        unsafe {
            xsk.umem
                .data_mut(&mut xsk.descs[i])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();
        }
    }

    assert_eq!(
        unsafe { xsk.tx_q.produce_and_wakeup(&xsk.descs[..nb]).unwrap() },
        nb
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_exact_on_fresh_queue_is_zero() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        assert_eq!(xsk1.rx_q.nb_avail_exact(), 0);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_exact_sees_late_arrivals() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        assert_eq!(unsafe { xsk2.fq.produce(&xsk2.descs[..2]) }, 2);

        // Nothing is consumed in between, so a count read from a
        // cached producer position would still report one here.
        send(&mut xsk1, 1);

        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == 1);

        // The kernel drops a zero length frame, so the second packet
        // has to be written before it is sent.
        unsafe {
            xsk1.umem
                .data_mut(&mut xsk1.descs[1])
                .cursor()
                .write_all(&ETHERNET_PACKET[..])
                .unwrap();
        }

        assert_eq!(
            unsafe { xsk1.tx_q.produce_and_wakeup(&xsk1.descs[1..2]).unwrap() },
            1
        );

        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == 2);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_exact_is_idempotent() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        assert_eq!(unsafe { xsk2.fq.produce(&xsk2.descs[..2]) }, 2);

        send(&mut xsk1, 2);

        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == 2);

        for _ in 0..3 {
            assert_eq!(xsk2.rx_q.nb_avail_exact(), 2);
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_exact_does_not_consume() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        assert_eq!(unsafe { xsk2.fq.produce(&xsk2.descs[..2]) }, 2);

        let mut fq_addrs = xsk2.descs[..2].iter().map(|d| d.addr()).collect::<Vec<_>>();

        send(&mut xsk1, 2);

        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == 2);

        assert_eq!(unsafe { xsk2.rx_q.consume(&mut xsk2.descs[..2]) }, 2);

        let mut rx_addrs = xsk2.descs[..2].iter().map(|d| d.addr()).collect::<Vec<_>>();

        fq_addrs.sort();
        rx_addrs.sort();

        assert_eq!(rx_addrs, fq_addrs);

        for i in 0..2 {
            assert_eq!(xsk2.descs[i].lengths().data(), ETHERNET_PACKET.len() as u32);

            assert_eq!(
                unsafe { xsk2.umem.data(&xsk2.descs[i]).contents() },
                ETHERNET_PACKET
            );
        }
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_exact_after_partial_consume() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        assert_eq!(unsafe { xsk2.fq.produce(&xsk2.descs[..3]) }, 3);

        send(&mut xsk1, 3);

        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == 3);

        assert_eq!(unsafe { xsk2.rx_q.consume_one(&mut xsk2.descs[0]) }, 1);

        assert_eq!(xsk2.rx_q.nb_avail_exact(), 2);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_exact_reports_full_ring() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        let rx_q_size = RX_Q_SIZE as usize;

        assert_eq!(
            unsafe { xsk2.fq.produce(&xsk2.descs[..rx_q_size]) },
            rx_q_size
        );

        send(&mut xsk1, rx_q_size);

        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == RX_Q_SIZE);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_on_fresh_queue_is_zero() {
    fn test(dev1: (Xsk, PacketGenerator), _dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;

        assert_eq!(xsk1.rx_q.nb_avail(1), 0);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_is_capped_at_nb() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        assert_eq!(unsafe { xsk2.fq.produce(&xsk2.descs[..3]) }, 3);

        send(&mut xsk1, 3);

        // Waiting on the cached count would spin forever, since
        // libxdp stops refreshing the cache once it has anything to
        // give.
        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == 3);

        assert_eq!(xsk2.rx_q.nb_avail(2), 2);

        assert_eq!(xsk2.rx_q.nb_avail(5), 3);
    }

    build_configs_and_run_test(test).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn nb_avail_can_answer_from_a_stale_cache() {
    fn test(dev1: (Xsk, PacketGenerator), dev2: (Xsk, PacketGenerator)) {
        let mut xsk1 = dev1.0;
        let mut xsk2 = dev2.0;

        assert_eq!(unsafe { xsk2.fq.produce(&xsk2.descs[..3]) }, 3);

        // Leaves libxdp's cache holding one entry, which is what
        // stops it reloading later on.
        send(&mut xsk1, 1);

        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == 1);

        for i in 1..3 {
            unsafe {
                xsk1.umem
                    .data_mut(&mut xsk1.descs[i])
                    .cursor()
                    .write_all(&ETHERNET_PACKET[..])
                    .unwrap();
            }
        }

        assert_eq!(
            unsafe { xsk1.tx_q.produce_and_wakeup(&xsk1.descs[1..3]).unwrap() },
            2
        );

        // Nothing on this side refreshes the cache, so there is no
        // condition to wait on: the count below reads one whether the
        // packets have arrived or not, and the sleep only makes it
        // likely that they have.
        thread::sleep(Duration::from_secs(1));

        assert_eq!(xsk2.rx_q.nb_avail(RX_Q_SIZE), 1);

        wait_until(WAIT_TIMEOUT, || xsk2.rx_q.nb_avail_exact() == 3);
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
