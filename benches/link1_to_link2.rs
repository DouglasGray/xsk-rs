use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rust_xsk::poll;

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

const FRAME_COUNT: u32 = 4096;
const PROD_Q_SIZE: u32 = 4096;
const CONS_Q_SIZE: u32 = 4096;
const MS_TIMEOUT: i32 = 10;

fn link1_to_link2_single_thread(num_packets: u64, dev1: &mut SocketState, dev2: &mut SocketState) {
    let mut dev1_frames = dev1.umem.frame_descs().to_vec();
    let mut dev2_frames = dev2.umem.frame_descs().to_vec();

    // Populate fill queue
    dev1.fill_q
        .produce_and_wakeup(
            &dev1_frames[..(PROD_Q_SIZE as usize)],
            dev1.socket.fd(),
            MS_TIMEOUT,
        )
        .unwrap();

    // Populate tx queue
    let mut total_pkts_sent = dev2
        .tx_q
        .produce_and_wakeup(&dev2_frames[..(PROD_Q_SIZE as usize)])
        .unwrap();

    let mut total_pkts_rcvd = 0;
    let mut total_pkts_consumed = 0;

    while total_pkts_sent < num_packets
        || total_pkts_rcvd < total_pkts_sent
        || total_pkts_consumed < total_pkts_sent
    {
        while total_pkts_rcvd < total_pkts_sent {
            if dev2.tx_q.needs_wakeup() {
                dev2.tx_q.wakeup().unwrap();
            }

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

        if total_pkts_sent < num_packets || total_pkts_consumed < total_pkts_sent {
            // Handle tx
            match dev2.comp_q.consume(&mut dev2_frames[..]) {
                0 => {
                    if dev2.tx_q.needs_wakeup() {
                        dev2.tx_q.wakeup().unwrap();
                    }
                }
                pkts_sent => {
                    if total_pkts_sent < num_packets {
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
                            if dev2.tx_q.needs_wakeup() {
                                dev2.tx_q.wakeup().unwrap();
                            }
                        }

                        total_pkts_sent += pkts_sent;
                    }

                    total_pkts_consumed += pkts_sent;

                    //println!("Total packets sent: {}", total_pkts_sent);
                }
            }
        }
    }
}

fn runner(c: &mut Criterion, mut dev1: SocketState, mut dev2: SocketState) {
    let mut group = c.benchmark_group("link1_to_link2_single_thread");

    for num_packets in [100_000].iter() {
        group.throughput(Throughput::Elements(*num_packets as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_packets),
            num_packets,
            |b, &num_pkts| {
                b.iter(|| link1_to_link2_single_thread(num_pkts, &mut dev1, &mut dev2));
            },
        );
    }
    group.finish();
}

pub fn xsk_benchmark(c: &mut Criterion) {
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

    let bench_fn = |dev1, dev2| {
        runner(c, dev1, dev2);
    };

    setup::run_bench(Some(umem_config), Some(socket_config), bench_fn);
}

criterion_group!(benches, xsk_benchmark);
criterion_main!(benches);
