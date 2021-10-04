#[allow(dead_code)]
mod setup;
use setup::{veth_setup, VethDevConfig};

use serial_test::serial;
use std::{convert::TryInto, io::Write};
use xsk_rs::{
    config::{SocketConfig, UmemConfig},
    socket::{RxQueue, Socket, TxQueue},
    umem::{frame::Frame, CompQueue, FillQueue, Umem},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn shared_umem_returns_new_fq_and_cq_when_sockets_are_bound_to_different_devices() {
    let inner = move |dev1_config: VethDevConfig, dev2_config: VethDevConfig| {
        let (umem, mut frames) =
            Umem::new(UmemConfig::default(), 64.try_into().unwrap(), false).unwrap();

        let (mut sender_tx_q, _sender_rx_q, sender_fq_and_cq) = Socket::new(
            SocketConfig::default(),
            &umem,
            &dev1_config.if_name().parse().unwrap(),
            0,
        )
        .unwrap();

        let (_sender_fq, mut sender_cq) = sender_fq_and_cq.unwrap();

        let (_receiver_tx_q, mut receiver_rx_q, receiver_fq_and_cq) = Socket::new(
            SocketConfig::default(),
            &umem,
            &dev2_config.if_name().parse().unwrap(),
            0,
        )
        .unwrap();

        let (mut receiver_fq, _receiver_cq) = receiver_fq_and_cq.unwrap();

        send_and_receive_pkt(
            &mut frames,
            (&mut sender_tx_q, &mut sender_cq),
            (&mut receiver_fq, &mut receiver_rx_q),
            "hello".as_bytes(),
        );
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

        let (_sender_tx_q, _sender_rx_q, sender_fq_and_cq) = Socket::new(
            SocketConfig::default(),
            &umem,
            &dev1_config.if_name().parse().unwrap(),
            0,
        )
        .unwrap();

        assert!(sender_fq_and_cq.is_some());

        let (_receiver_tx_q, _receiver_rx_q, receiver_fq_and_cq) = Socket::new(
            SocketConfig::default(),
            &umem,
            &dev1_config.if_name().parse().unwrap(),
            0,
        )
        .unwrap();

        assert!(receiver_fq_and_cq.is_none());
    };

    let (dev1_config, dev2_config) = setup::default_veth_dev_configs();

    veth_setup::run_with_veth_pair(inner, dev1_config, dev2_config)
        .await
        .unwrap();
}

fn send_and_receive_pkt(
    frames: &mut [Frame],
    sender: (&mut TxQueue, &mut CompQueue),
    receiver: (&mut FillQueue, &mut RxQueue),
    pkt: &[u8],
) {
    unsafe {
        assert_eq!(
            receiver
                .0
                .produce_and_wakeup(&frames[0..1], receiver.1.fd_mut(), 100)
                .unwrap(),
            1
        );

        frames[1].data_mut().cursor().write_all(pkt).unwrap();

        loop {
            if sender.0.produce_and_wakeup(&frames[1..2]).unwrap() == 1 {
                break;
            }
        }

        loop {
            if receiver.1.poll_and_consume(&mut frames[2..3], 100).unwrap() == 1 {
                break;
            }
        }

        assert_eq!(sender.1.consume(&mut frames[3..4]), 1);

        // Check that:
        // 1. Data received matches
        // 2. Address consumed in comp queue is address of frame written to
        // 3. Address consumed in rx queue is address of frame added to fill queue

        assert_eq!(frames[1].data().contents(), pkt);
        assert_eq!(frames[3].addr(), frames[1].addr());
        assert_eq!(frames[2].addr(), frames[0].addr());
    }
}
