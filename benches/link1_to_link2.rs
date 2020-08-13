use criterion::{criterion_group, criterion_main, Criterion};
use std::thread;
use tokio::runtime::Runtime;

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

const PROD_Q_SIZE: u32 = 2048;
const CONS_Q_SIZE: u32 = 2048;
const MS_TIMEOUT: i32 = 1000;

fn rx(num_packets: u64, mut rx_socket: SocketState) {
    // Receiver thread
    let mut total_pkts_recvd = 0;

    let mut rx_frames = rx_socket.umem.frame_descs().to_vec();

    // Initialise the fill queue
    let frames_produced = rx_socket.fill_q.produce_and_wakeup(
        &rx_frames[..],
        rx_socket.socket.file_descriptor(),
        MS_TIMEOUT,
    );

    assert_eq!(frames_produced, PROD_Q_SIZE as u64);

    while total_pkts_recvd < num_packets {
        // Try read some frames
        let pkts_recvd = rx_socket
            .rx_q
            .wakeup_and_consume(&mut rx_frames[..], MS_TIMEOUT)
            .unwrap();

        if pkts_recvd > 0 {
            // If we've read some frames then add them back to the fill queue
            rx_socket.fill_q.produce_and_wakeup(
                &rx_frames[..(pkts_recvd as usize)],
                rx_socket.socket.file_descriptor(),
                MS_TIMEOUT,
            );

            total_pkts_recvd += pkts_recvd;
        }
    }
}

fn tx(num_packets: u64, tx_socket: SocketState) {
    // Sender thread
    let mut total_pkts_sent = 0;

    let mut tx_frames = tx_socket.umem.frame_descs().to_vec();

    // Initialise the tx queue
    let frames_produced = tx_socket.tx_q.produce_and_wakeup(&tx_frames[..]);

    assert_eq!(frames_produced, PROD_Q_SIZE as u64);

    while total_pkts_sent < num_packets {
        // Check the completion queue
        let pkts_sent = tx_socket.comp_q.consume(&mut tx_frames[..]);

        if pkts_sent > 0 {
            // If we've sent some frames then add them back to the tx queue
            tx_socket
                .tx_q
                .produce_and_wakeup(&tx_frames[..(pkts_sent as usize)]);

            total_pkts_sent += pkts_sent;
        }
    }
}

fn link1_to_link2(num_packets: u64) {
    let umem_config = UmemConfigBuilder {
        frame_count: 4096,
        frame_size: 2048,
        fill_queue_size: PROD_Q_SIZE,
        comp_queue_size: CONS_Q_SIZE,
        ..UmemConfigBuilder::default()
    }
    .build();

    let socket_config = SocketConfigBuilder {
        tx_queue_size: PROD_Q_SIZE,
        rx_queue_size: CONS_Q_SIZE,
        ..SocketConfigBuilder::default()
    }
    .build();

    fn bench(num_packets: u64, dev1: SocketState, dev2: SocketState) {
        let rx_handle = thread::spawn(move || rx(num_packets, dev1));
        unimplemented!()
    }

    let mut runtime = Runtime::new().expect("Failed to create tokio runtime");

    runtime.block_on(setup::run_bench(
        Some(umem_config),
        Some(socket_config),
        bench,
        num_packets,
    ))
}

pub fn xsk_benchmark(c: &mut Criterion) {
    c.bench_function("link1_to_link2", |b| b.iter(|| link1_to_link2()));
}

criterion_group!(benches, xsk_benchmark);
criterion_main!(benches);
