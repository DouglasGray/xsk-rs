use criterion::{criterion_group, criterion_main, Criterion};
use std::slice;
use tokio::runtime::Runtime;

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

fn rx(num_packets: u32, mut rx_socket: SocketState) {
    // Receiver thread
    let pkts_recvd = 0;

    let mut fill_q_frames = rx_socket.umem.frame_descs().to_vec();
    let mut rx_q_frames = rx_socket.umem.frame_descs().to_vec();

    // Initialise the fill queue
    let fill_q_ix = rx_socket.fill_q.produce(&fill_q_frames[..]);

    while pkts_recvd < num_packets {
        // Try read some frames
        let frames_recvd = rx_socket
            .rx_q
            .wakeup_and_consume(&mut rx_q_frames[..], 1000)
            .unwrap();

        if frames_recvd > 0 {
            // If we've read some frames then add them back to the fill queue
            fill_q_frames[(fill_q_ix - frames_recvd)..fill_q_ix]
                .copy_from_slice(rx_q_frames[..frames_recvd]);
            let frames_produced = rx_socket.fill_q.produce(&fill_q_frames[..]);
        }
    }
}

fn tx(num_packets: u32, tx_socket: SocketState) {
    unimplemented!();
}

fn link1_to_link2() {
    let umem_config = UmemConfigBuilder {
        frame_count: 4096,
        frame_size: 2048,
        fill_queue_size: 2048,
        comp_queue_size: 2048,
        ..UmemConfigBuilder::default()
    }
    .build();

    let socket_config = SocketConfigBuilder {
        tx_queue_size: 2048,
        rx_queue_size: 2048,
        ..SocketConfigBuilder::default()
    }
    .build();

    fn bench(dev1: SocketState, dev2: SocketState) {
        //
        unimplemented!()
    }

    let mut runtime = Runtime::new().expect("Failed to create tokio runtime");

    runtime.block_on(setup::run_bench(
        Some(umem_config),
        Some(socket_config),
        bench,
    ))
}

pub fn xsk_benchmark(c: &mut Criterion) {
    c.bench_function("link1_to_link2", |b| b.iter(|| link1_to_link2()));
}

criterion_group!(benches, xsk_benchmark);
criterion_main!(benches);
