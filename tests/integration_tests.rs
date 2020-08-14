use rust_xsk::{socket::Config as SocketConfig, umem::Config as UmemConfig};

mod setup;

use setup::{SocketConfigBuilder, SocketState, UmemConfigBuilder};

const FRAME_COUNT: u32 = 256;
const PROD_Q_SIZE: u32 = 8;
const CONS_Q_SIZE: u32 = 8;

fn build_configs() -> (Option<UmemConfig>, Option<SocketConfig>) {
    let umem_config = UmemConfigBuilder {
        frame_count: FRAME_COUNT,
        frame_size: 4096,
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

    (Some(umem_config), Some(socket_config))
}

#[tokio::test]
async fn rx_drop() {
    fn test_fn(mut dev1: SocketState, mut dev2: SocketState) {
        let mut dev1_frames = dev1.umem.frame_descs().to_vec();
        let mut dev2_frames = dev2.umem.frame_descs().to_vec();

        let cnt = 1_000;

        // Populate fill queue
        assert_eq!(
            dev1.fill_q
                .produce_and_wakeup(&dev1_frames[..], dev1.socket.fd(), 100)
                .unwrap(),
            PROD_Q_SIZE as u64
        );

        // Populate tx queue
        assert_eq!(
            dev2.tx_q.produce_and_wakeup(&dev2_frames[..]).unwrap(),
            PROD_Q_SIZE as u64
        );

        let mut total_pkts_sent = 0;
        let mut total_pkts_received = 0;

        while total_pkts_sent < cnt || total_pkts_received < cnt {
            if total_pkts_received < cnt {
                // Handle rx
                match dev1
                    .rx_q
                    .wakeup_and_consume(&mut dev1_frames[..], 100)
                    .unwrap()
                {
                    0 => {
                        // No packets consumed, wake up fill queue if required
                        if dev1.fill_q.needs_wakeup() {
                            dev1.fill_q.wakeup(dev1.socket.fd(), 100).unwrap();
                        }
                    }
                    pkts_recvd => {
                        // Add consumed frames back to the fill queue
                        println!("Received {} packets, adding back to fill queue", pkts_recvd);

                        let mut filled = 0;

                        while filled < pkts_recvd {
                            let produced = dev1
                                .fill_q
                                .produce_and_wakeup(
                                    &dev1_frames[(filled as usize)..(pkts_recvd as usize)],
                                    dev1.socket.fd(),
                                    100,
                                )
                                .unwrap();

                            if dev1.fill_q.needs_wakeup() {
                                dev1.fill_q.wakeup(dev1.socket.fd(), 100).unwrap();
                            }

                            if produced > 0 {
                                println!("Adding {} frames back to the fill queue", produced);
                                filled += produced;
                            }
                        }

                        total_pkts_received += pkts_recvd;

                        println!("Added {} packets back to fill queue", filled);
                    }
                }
            }

            if total_pkts_sent < cnt {
                // Handle tx
                match dev2.comp_q.consume(&mut dev2_frames[..]) {
                    0 => {
                        dev2.socket.wakeup().unwrap();
                    }
                    pkts_sent => {
                        // Add consumed frames back to the tx queue
                        println!("Sent {} packets, adding back to tx queue", pkts_sent);

                        let mut filled = 0;

                        while filled < pkts_sent {
                            filled += dev2
                                .tx_q
                                .produce_and_wakeup(
                                    &dev2_frames[(filled as usize)..(pkts_sent as usize)],
                                )
                                .unwrap()
                        }

                        total_pkts_sent += pkts_sent;
                    }
                }
            }
        }
    }

    let (umem_config, socket_config) = build_configs();

    setup::run_test(umem_config, socket_config, test_fn).await;
}
