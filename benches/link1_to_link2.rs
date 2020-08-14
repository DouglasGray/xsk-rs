use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rust_xsk::poll;
use tokio::runtime::Runtime;

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

const FRAME_COUNT: u32 = 4096;
const PROD_Q_SIZE: u32 = 4096;
const CONS_Q_SIZE: u32 = 4096;
const MS_TIMEOUT: i32 = 100;

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

    fn bench(num_packets: u64, mut dev1: SocketState, mut dev2: SocketState) {
        let mut dev1_frames = dev1.umem.frame_descs().to_vec();
        let mut dev2_frames = dev2.umem.frame_descs().to_vec();

        // Populate fill queue
        assert_eq!(
            dev1.fill_q
                .produce_and_wakeup(
                    &dev1_frames[..(PROD_Q_SIZE as usize)],
                    dev1.socket.fd(),
                    MS_TIMEOUT
                )
                .unwrap(),
            PROD_Q_SIZE as u64
        );

        // Populate tx queue
        assert_eq!(
            dev2.tx_q
                .produce_and_wakeup(&dev2_frames[..(PROD_Q_SIZE as usize)])
                .unwrap(),
            PROD_Q_SIZE as u64
        );

        let mut total_pkts_sent = 0;
        let mut total_pkts_rcvd = 0;

        println!("** Starting benchmark w/ {} packets ***", num_packets);

        while total_pkts_sent < num_packets || total_pkts_rcvd < num_packets {
            if total_pkts_rcvd < num_packets {
                // Handle rx
                match dev1
                    .rx_q
                    .wakeup_and_consume(&mut dev1_frames[..], MS_TIMEOUT)
                    .unwrap()
                {
                    0 => {
                        // No packets consumed, wake up fill queue if required
                        if dev1.fill_q.needs_wakeup() {
                            dev1.fill_q.wakeup(dev1.socket.fd(), MS_TIMEOUT).unwrap();
                        }
                    }
                    pkts_recvd => {
                        // Add frames back to fill queue
                        while dev1
                            .fill_q
                            .produce_and_wakeup(
                                &dev1_frames[..(pkts_recvd as usize)],
                                dev1.socket.fd(),
                                MS_TIMEOUT,
                            )
                            .unwrap()
                            != pkts_recvd
                        {
                            if dev1.fill_q.needs_wakeup() {
                                dev1.fill_q.wakeup(dev1.socket.fd(), MS_TIMEOUT).unwrap();
                            }
                        }

                        total_pkts_rcvd += pkts_recvd;

                        //println!("Total packets received: {}", total_pkts_rcvd);
                    }
                }
            }

            if total_pkts_sent < num_packets {
                // Handle tx
                match dev2.comp_q.consume(&mut dev2_frames[..]) {
                    0 => {
                        dev2.socket.wakeup().unwrap();
                    }
                    pkts_sent => {
                        // Add consumed frames back to the tx queue
                        while !poll::poll_write(dev2.socket.fd(), MS_TIMEOUT).unwrap() {
                            continue;
                        }

                        while dev2
                            .tx_q
                            .produce_and_wakeup(&dev2_frames[..(pkts_sent as usize)])
                            .unwrap()
                            != pkts_sent
                        {
                            dev2.socket.wakeup().unwrap();
                        }

                        total_pkts_sent += pkts_sent;

                        //println!("Total packets sent: {}", total_pkts_sent);
                    }
                }
            }
        }
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

    for num_packets in [1_000].iter() {
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
