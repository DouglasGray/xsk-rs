#[allow(dead_code)]
mod setup;
use setup::{veth_setup, VethDevConfig, Xsk, ETHERNET_PACKET};

use serial_test::serial;
use std::{convert::TryInto, io::Write};
use xsk_rs::{
    config::{LibxdpFlags, SocketConfig, UmemConfig},
    Socket, Umem,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn shared_umem_returns_new_fq_and_cq_when_sockets_are_bound_to_different_devices() {
    let inner = move |dev1_config: VethDevConfig, dev2_config: VethDevConfig| {
        let frame_count = 64;

        let (umem, descs) = Umem::new(
            UmemConfig::default(),
            frame_count.try_into().unwrap(),
            false,
        )
        .unwrap();

        let mut sender_descs = descs;
        let receiver_descs = sender_descs.drain((frame_count / 2) as usize..).collect();

        let (sender_tx_q, sender_rx_q, sender_fq_and_cq) = unsafe {
            Socket::new(
                SocketConfig::default(),
                &umem,
                &dev1_config.if_name().parse().unwrap(),
                0,
            )
        }
        .unwrap();

        let (sender_fq, sender_cq) = sender_fq_and_cq.unwrap();

        let mut sender = Xsk {
            umem: umem.clone(),
            fq: sender_fq,
            cq: sender_cq,
            tx_q: sender_tx_q,
            rx_q: sender_rx_q,
            descs: sender_descs,
        };

        let (receiver_tx_q, receiver_rx_q, receiver_fq_and_cq) = unsafe {
            Socket::new(
                SocketConfig::default(),
                &umem,
                &dev2_config.if_name().parse().unwrap(),
                0,
            )
        }
        .unwrap();

        let (receiver_fq, receiver_cq) = receiver_fq_and_cq.unwrap();

        let mut receiver = Xsk {
            umem,
            fq: receiver_fq,
            cq: receiver_cq,
            tx_q: receiver_tx_q,
            rx_q: receiver_rx_q,
            descs: receiver_descs,
        };

        send_and_receive_pkt(&mut sender, &mut receiver, &ETHERNET_PACKET[..]);
    };

    let (dev1_config, dev2_config) = setup::default_veth_dev_configs();

    veth_setup::run_with_veth_pair(inner, dev1_config, dev2_config)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn shared_umem_does_not_return_new_fq_and_cq_when_sockets_are_bound_to_same_device() {
    let inner = move |dev1_config: VethDevConfig, _dev2_config: VethDevConfig| {
        let (umem, _frames) =
            Umem::new(UmemConfig::default(), 64.try_into().unwrap(), false).unwrap();

        let (_sender_tx_q, _sender_rx_q, sender_fq_and_cq) = unsafe {
            Socket::new(
                SocketConfig::builder()
                    .libxdp_flags(LibxdpFlags::XSK_LIBXDP_FLAGS_INHIBIT_PROG_LOAD)
                    .build(),
                &umem,
                &dev1_config.if_name().parse().unwrap(),
                0,
            )
        }
        .unwrap();

        assert!(sender_fq_and_cq.is_some());

        let (_receiver_tx_q, _receiver_rx_q, receiver_fq_and_cq) = unsafe {
            Socket::new(
                SocketConfig::builder()
                    .libxdp_flags(LibxdpFlags::XSK_LIBXDP_FLAGS_INHIBIT_PROG_LOAD)
                    .build(),
                &umem,
                &dev1_config.if_name().parse().unwrap(),
                0,
            )
        }
        .unwrap();

        assert!(receiver_fq_and_cq.is_none());
    };

    let (dev1_config, dev2_config) = setup::default_veth_dev_configs();

    veth_setup::run_with_veth_pair(inner, dev1_config, dev2_config)
        .await
        .unwrap();
}

#[tokio::test]
#[serial]
async fn writing_to_frame_and_reading_works_as_expected() {
    let (umem, mut descs) = Umem::new(
        UmemConfig::builder().frame_headroom(32).build().unwrap(),
        64.try_into().unwrap(),
        false,
    )
    .unwrap();

    unsafe {
        let (mut h, mut d) = umem.frame_mut(&mut descs[0]);

        h.cursor().write_all(b"hello").unwrap();
        d.cursor().write_all(b"world").unwrap();

        assert_eq!(umem.headroom(&descs[0]).contents(), b"hello");
        assert_eq!(umem.headroom_mut(&mut descs[0]).contents(), b"hello");

        assert_eq!(umem.data(&descs[0]).contents(), b"world");
        assert_eq!(umem.data_mut(&mut descs[0]).contents(), b"world");
    }
}

fn send_and_receive_pkt(sender: &mut Xsk, receiver: &mut Xsk, pkt: &[u8]) {
    unsafe {
        assert_eq!(
            receiver
                .fq
                .produce_and_wakeup(&receiver.descs[0..1], receiver.rx_q.fd_mut(), 100)
                .unwrap(),
            1
        );

        sender
            .umem
            .data_mut(&mut sender.descs[0])
            .cursor()
            .write_all(pkt)
            .unwrap();

        loop {
            if sender.tx_q.produce_and_wakeup(&sender.descs[..1]).unwrap() == 1 {
                break;
            }
        }

        loop {
            if receiver
                .rx_q
                .poll_and_consume(&mut receiver.descs[1..2], 100)
                .unwrap()
                == 1
            {
                break;
            }
        }

        assert_eq!(sender.cq.consume(&mut sender.descs[1..2]), 1);

        // Check that:
        // 1. Data received matches
        // 2. Address consumed in rx queue is address of frame added to fill queue
        // 3. Address consumed in comp queue is address of frame written to

        assert_eq!(receiver.umem.data(&receiver.descs[1]).contents(), pkt);
        assert_eq!(receiver.descs[1].addr(), receiver.descs[0].addr());
        assert_eq!(sender.descs[1].addr(), sender.descs[0].addr());
    }
}
