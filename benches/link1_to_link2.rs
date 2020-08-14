use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rust_xsk::{poll, socket::BindFlags};
use std::thread;
use tokio::runtime::Runtime;

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

const FRAME_COUNT: u32 = 2048;
const PROD_Q_SIZE: u32 = 2048;
const CONS_Q_SIZE: u32 = 2048;
const MS_TIMEOUT: i32 = 100;

fn rx(num_packets: u64, mut rx_socket: SocketState) {
    // Receiver thread
    let mut total_pkts_recvd = 0;

    let mut rx_frames = rx_socket.umem.frame_descs().to_vec();

    // Initialise the fill queue
    let frames_produced = rx_socket
        .fill_q
        .produce_and_wakeup(
            &rx_frames[..],
            rx_socket.socket.file_descriptor(),
            MS_TIMEOUT,
        )
        .unwrap();

    assert_eq!(frames_produced, PROD_Q_SIZE as u64);

    while total_pkts_recvd < num_packets {
        // Try read some frames
        match rx_socket
            .rx_q
            .wakeup_and_consume(&mut rx_frames[..], MS_TIMEOUT)
            .unwrap()
        {
            0 => {
                if rx_socket.fill_q.needs_wakeup() {
                    rx_socket
                        .fill_q
                        .wakeup(rx_socket.socket.file_descriptor(), MS_TIMEOUT)
                        .unwrap();
                }
            }
            pkts_recvd => {
                // If we've read some frames then add them back to the fill queue
                let mut filled = 0;

                println!("Packets received: {}", pkts_recvd);
                while filled < pkts_recvd {
                    filled += rx_socket
                        .fill_q
                        .produce_and_wakeup(
                            &rx_frames[(filled as usize)..(pkts_recvd as usize)],
                            rx_socket.socket.file_descriptor(),
                            MS_TIMEOUT,
                        )
                        .unwrap();
                }

                total_pkts_recvd += pkts_recvd;

                println!("Total packets received: {}", total_pkts_recvd);
            }
        }
    }
}

fn tx(num_packets: u64, mut tx_socket: SocketState) {
    // Sender thread
    let mut total_pkts_sent = 0;

    let mut tx_frames = tx_socket.umem.frame_descs().to_vec();

    while !poll::poll_write(tx_socket.socket.file_descriptor(), MS_TIMEOUT).unwrap() {
        continue;
    }

    // Initialise the tx queue
    let frames_produced = tx_socket.tx_q.produce_and_wakeup(&tx_frames[..]).unwrap();

    assert_eq!(frames_produced, PROD_Q_SIZE as u64);

    while total_pkts_sent < num_packets {
        // Check the completion queue
        match tx_socket.comp_q.consume(&mut tx_frames[..]) {
            0 => {
                tx_socket.socket.wakeup().unwrap();
            }
            pkts_sent => {
                // If we've sent some frames then add them back to the tx queue
                let mut filled = 0;

                println!("Packets sent: {}", pkts_sent);

                while !poll::poll_write(tx_socket.socket.file_descriptor(), MS_TIMEOUT).unwrap() {
                    continue;
                }

                while filled < pkts_sent {
                    filled += tx_socket
                        .tx_q
                        .produce_and_wakeup(&tx_frames[(filled as usize)..(pkts_sent as usize)])
                        .unwrap();
                }

                total_pkts_sent += pkts_sent;

                println!("Total packets sent: {}", total_pkts_sent);
            }
        }
    }
}

fn link1_to_link2(num_packets: u64) {
    let umem_config = UmemConfigBuilder {
        frame_count: FRAME_COUNT,
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
        let handles = vec![
            thread::spawn(move || rx(num_packets, dev1)),
            thread::spawn(move || tx(num_packets, dev2)),
        ];

        let _ = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<()>();
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
    let mut group = c.benchmark_group("link1_to_link2");

    for num_packets in [10_000].iter() {
        group.throughput(Throughput::Bytes(*num_packets as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_packets),
            num_packets,
            |b, &num_pkts| {
                b.iter(|| link1_to_link2(num_pkts));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, xsk_benchmark);
criterion_main!(benches);
