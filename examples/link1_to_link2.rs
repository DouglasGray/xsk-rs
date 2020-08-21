use std::{convert::TryInto, num::NonZeroU32, thread, time::Instant};
use tokio::{
    runtime::Runtime,
    sync::oneshot::{self, error::TryRecvError},
};
use xsk_rs::{
    socket, BindFlags, CompQueue, FillQueue, FrameDesc, LibbpfFlags, RxQueue, Socket, SocketConfig,
    TxQueue, Umem, UmemConfig, XdpFlags,
};

mod setup;

const FRAME_COUNT: u32 = 4096;
const FRAME_SIZE: u32 = 2048;
const PROD_Q_SIZE: u32 = 4096;
const CONS_Q_SIZE: u32 = 4096;
const MS_TIMEOUT: i32 = 10;
const MSG_SIZE: u32 = 64;
const NUM_PACKETS_TO_SEND: usize = 5_000_000;

struct SocketState<'umem> {
    umem: Umem<'umem>,
    fill_q: FillQueue<'umem>,
    comp_q: CompQueue<'umem>,
    tx_q: TxQueue<'umem>,
    rx_q: RxQueue<'umem>,
    frame_descs: Vec<FrameDesc>,
}

fn build_socket_and_umem<'a, 'umem>(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    if_name: &'a str,
    queue_id: u32,
) -> SocketState {
    let (mut umem, fill_q, comp_q, frame_descs) = Umem::builder(umem_config)
        .create_mmap()
        .expect(format!("Failed to create mmap area for {}", if_name).as_str())
        .create_umem()
        .expect(format!("Failed to create umem for {}", if_name).as_str());

    let (tx_q, rx_q) = Socket::new(socket_config, &mut umem, if_name, queue_id)
        .expect(format!("Failed to build socket for {}", if_name).as_str());

    SocketState {
        umem,
        fill_q,
        comp_q,
        tx_q,
        rx_q,
        frame_descs,
    }
}

fn generate_random_bytes(len: u32) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}

// Generate random messages of size `MSG_SIZE` and send them through dev2 to be received by dev1
// This is single threaded so will handle the send and receive process alternately
fn link1_to_link2_single_thread(dev1: &mut SocketState, dev2: &mut SocketState) {
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

    // Populate tx queue
    let mut total_pkts_sent = dev2
        .tx_q
        .produce_and_wakeup(&dev2_frames[..(PROD_Q_SIZE as usize)])
        .unwrap();

    let mut total_pkts_rcvd = 0;
    let mut total_pkts_consumed = 0;

    while total_pkts_sent < NUM_PACKETS_TO_SEND
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
                }
            }
        }

        if total_pkts_sent < NUM_PACKETS_TO_SEND || total_pkts_consumed < total_pkts_sent {
            // Handle tx
            match dev2.comp_q.consume(&mut dev2_frames[..]) {
                0 => {
                    if dev2.tx_q.needs_wakeup() {
                        dev2.tx_q.wakeup().unwrap();
                    }
                }
                pkts_sent => {
                    if total_pkts_sent < NUM_PACKETS_TO_SEND {
                        // Data is still contained in the frames so just set the descriptor's length
                        for desc in dev2_frames[..pkts_sent].iter_mut() {
                            desc.set_len(MSG_SIZE);
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
                }
            }
        }
    }
}

fn main() {
    let dev1_if_name = String::from("xsk_ex_dev1");
    let dev2_if_name = String::from("xsk_ex_dev2");

    let (startup_w, mut startup_r) = oneshot::channel();
    let (shutdown_w, shutdown_r) = oneshot::channel();

    let dev1_if_name_clone = dev1_if_name.clone();
    let dev2_if_name_clone = dev2_if_name.clone();

    // Set up the veth link
    let veth_handle = thread::spawn(move || {
        let mut runtime = Runtime::new().unwrap();

        runtime.block_on(setup::run_veth_link(
            &dev1_if_name_clone,
            &dev2_if_name_clone,
            startup_w,
            shutdown_r,
        ))
    });

    println!("Setting up veth link");

    loop {
        match startup_r.try_recv() {
            Ok(_) => break,
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Closed) => panic!("Failed to set up veth link"),
        }
    }

    println!("veth link is up");

    // Create umem and socket configs
    let umem_config = UmemConfig::new(
        NonZeroU32::new(FRAME_COUNT).unwrap(),
        NonZeroU32::new(FRAME_SIZE).unwrap(),
        PROD_Q_SIZE,
        CONS_Q_SIZE,
        0,
        false,
    )
    .unwrap();

    let socket_config = SocketConfig::new(
        CONS_Q_SIZE,
        PROD_Q_SIZE,
        LibbpfFlags::empty(),
        XdpFlags::empty(),
        BindFlags::empty(),
    )
    .unwrap();

    let mut dev1 =
        build_socket_and_umem(umem_config.clone(), socket_config.clone(), &dev1_if_name, 0);

    let mut dev2 = build_socket_and_umem(umem_config, socket_config, &dev2_if_name, 0);

    let now = Instant::now();

    println!("Processing {} messages", NUM_PACKETS_TO_SEND);

    // Copy over some bytes to dev2's umem
    for desc in dev2.frame_descs.iter_mut() {
        let bytes = generate_random_bytes(MSG_SIZE);
        let len = dev2.umem.copy_data_to_frame(&desc.addr(), &bytes).unwrap();
        desc.set_len(len.try_into().unwrap());
    }

    // Send messages
    link1_to_link2_single_thread(&mut dev1, &mut dev2);

    println!(
        "Seconds taken to send and receive {} messages: {}",
        NUM_PACKETS_TO_SEND,
        now.elapsed().as_secs()
    );

    // Wait for link to close
    if let Err(e) = shutdown_w.send(()) {
        eprintln!("veth link thread returned unexpectedly: {:?}", e);
    }

    veth_handle.join().unwrap();
}
