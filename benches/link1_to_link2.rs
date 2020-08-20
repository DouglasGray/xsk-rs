use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rust_xsk::socket;
use std::convert::TryInto;

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

const FRAME_COUNT: u32 = 4096;
const FRAME_SIZE: u32 = 2048;
const PROD_Q_SIZE: u32 = 4096;
const CONS_Q_SIZE: u32 = 4096;
const MS_TIMEOUT: i32 = 10;
const MSG_SIZE: u32 = 64;

fn generate_random_bytes(len: u32) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}

fn link1_to_link2_single_thread(num_packets: u64, dev1: &mut SocketState, dev2: &mut SocketState) {
    let dev1_frames = &mut dev1.frame_descs;
    let dev2_frames = &mut dev2.frame_descs;

    // Populate fill queue
    dev1.fill_q
        .produce_and_wakeup(
            &dev1_frames[..(PROD_Q_SIZE as usize)],
            dev1.rx_q.fd(),
            MS_TIMEOUT,
        )
        .unwrap();

    // Copy over some bytes to dev2's UMEM
    for desc in dev2_frames.iter_mut() {
        let bytes = generate_random_bytes(MSG_SIZE);
        let len = dev2.umem.copy_data_to_frame(&desc.addr(), &bytes).unwrap();
        desc.set_len(len.try_into().unwrap());
    }

    // Populate tx queue
    let mut total_pkts_sent = dev2
        .tx_q
        .produce_and_wakeup(&dev2_frames[..(PROD_Q_SIZE as usize)])
        .unwrap();

    let mut total_pkts_rcvd = 0;
    let mut total_pkts_consumed = 0;

    let num_packets: usize = num_packets.try_into().unwrap();

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
                        dev1.fill_q.wakeup(dev1.rx_q.fd(), MS_TIMEOUT).unwrap();
                    }
                }
                pkts_recvd => {
                    // Add frames back to fill queue
                    while dev1
                        .fill_q
                        .produce_and_wakeup(&dev1_frames[..pkts_recvd], dev1.rx_q.fd(), MS_TIMEOUT)
                        .unwrap()
                        != pkts_recvd
                    {
                        if dev1.fill_q.needs_wakeup() {
                            dev1.fill_q.wakeup(dev1.rx_q.fd(), MS_TIMEOUT).unwrap();
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
                        // Populate the frames with new data
                        for desc in dev2_frames[..pkts_sent].iter_mut() {
                            let bytes = generate_random_bytes(MSG_SIZE);
                            let len = dev2.umem.copy_data_to_frame(&desc.addr(), &bytes).unwrap();
                            desc.set_len(len.try_into().unwrap());
                        }

                        // Add consumed frames back to the tx queue
                        while !socket::poll_write(dev2.tx_q.fd(), MS_TIMEOUT).unwrap() {
                            continue;
                        }

                        while dev2
                            .tx_q
                            .produce_and_wakeup(&dev2_frames[..pkts_sent])
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
        frame_size: FRAME_SIZE,
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
