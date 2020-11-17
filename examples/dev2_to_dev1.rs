mod setup;

use clap::{App, Arg};
use etherparse::PacketBuilder;
use std::{
    cmp,
    convert::TryInto,
    io,
    net::Ipv4Addr,
    num::NonZeroU32,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Instant,
};
use tokio::{
    runtime::Runtime,
    sync::oneshot::{self, error::TryRecvError},
};
use xsk_rs::{
    socket, BindFlags, CompQueue, FillQueue, FrameDesc, LibbpfFlags, RxQueue, Socket, SocketConfig,
    TxQueue, Umem, UmemConfig, XdpFlags,
};

use setup::{LinkIpAddr, VethConfig};

const RX_Q_SIZE: u32 = 4096;
const TX_Q_SIZE: u32 = 4096;
const COMP_Q_SIZE: u32 = 4096;
const FILL_Q_SIZE: u32 = 4096 * 8;
const FRAME_SIZE: u32 = 2048;
const FRAME_COUNT: u32 = COMP_Q_SIZE + FILL_Q_SIZE;
const MS_TIMEOUT: i32 = 100;
const PAYLOAD_SIZE: u32 = 64;
const MAX_BATCH_SIZE: usize = 64;
const NUM_PACKETS_TO_SEND: usize = 5_000_000;

// Reqd for the multithreaded case to signal when all packets have been sent
static SENDER_DONE: AtomicBool = AtomicBool::new(false);

struct SocketState<'umem> {
    umem: Umem<'umem>,
    fill_q: FillQueue<'umem>,
    comp_q: CompQueue<'umem>,
    tx_q: TxQueue<'umem>,
    rx_q: RxQueue<'umem>,
    frame_descs: Vec<FrameDesc>,
}

fn build_socket_and_umem(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    if_name: &str,
    queue_id: u32,
) -> SocketState<'static> {
    let (mut umem, fill_q, comp_q, frame_descs) = Umem::builder(umem_config)
        .create_mmap()
        .expect(format!("Failed to create mmap area for {}", if_name).as_str())
        .create_umem()
        .expect(format!("Failed to create umem for {}", if_name).as_str());

    let (tx_q, rx_q) = Socket::new(socket_config, &mut umem, &if_name, queue_id)
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

fn generate_eth_frame(veth_config: &VethConfig, payload_len: u32) -> Vec<u8> {
    let builder = PacketBuilder::ethernet2(
        veth_config.dev1_addr().clone(), // src mac
        veth_config.dev2_addr().clone(), // dst mac
    )
    .ipv4(
        veth_config.dev1_ip_addr().octets(), // src ip
        veth_config.dev2_ip_addr().octets(), // dst ip
        20,                                  // time to live
    )
    .udp(
        1234, // src port
        1234, // dst port
    );

    let payload = generate_random_bytes(payload_len);

    let mut result = Vec::<u8>::with_capacity(builder.size(payload.len()));

    builder.write(&mut result, &payload).unwrap();

    result
}

// Generate random messages of size `MSG_SIZE` and send them through dev2 to be received by dev1
// This is single threaded so will handle the send and receive process alternately
fn dev2_to_dev1_single_thread(mut dev1: SocketState, mut dev2: SocketState, msg_size: u32) {
    let dev1_frames = &mut dev1.frame_descs;
    let dev2_frames = &mut dev2.frame_descs;

    let start = Instant::now();

    // Populate fill queue
    let frames_filled = dev1
        .fill_q
        .produce_and_wakeup(
            &dev1_frames[..FILL_Q_SIZE as usize],
            dev1.rx_q.fd(),
            MS_TIMEOUT,
        )
        .unwrap();

    assert_eq!(frames_filled, FILL_Q_SIZE as usize);

    // Populate tx queue
    let mut total_pkts_sent = dev2.tx_q.produce(&dev2_frames[..MAX_BATCH_SIZE]);

    assert_eq!(total_pkts_sent, MAX_BATCH_SIZE);

    let mut total_pkts_rcvd = 0;
    let mut total_pkts_consumed = 0;

    while total_pkts_sent < NUM_PACKETS_TO_SEND
        || total_pkts_rcvd < total_pkts_sent
        || total_pkts_consumed < total_pkts_sent
    {
        while total_pkts_rcvd < total_pkts_sent {
            // In copy mode tx is driven by a syscall, so we need to wakeup the kernel
            // with a call to either sendto() or poll() (wakeup() below uses sendto()).
            if dev2.tx_q.needs_wakeup() {
                dev2.tx_q.wakeup().unwrap();
            }

            // Handle rx
            match dev1
                .rx_q
                .poll_and_consume(&mut dev1_frames[..], MS_TIMEOUT)
                .unwrap()
            {
                0 => {
                    // No packets consumed, wake up fill queue if required
                    if dev1.fill_q.needs_wakeup() {
                        dev1.fill_q.wakeup(dev1.rx_q.fd(), MS_TIMEOUT).unwrap();
                    }
                }
                pkts_rcvd => {
                    // Add frames back to fill queue
                    while dev1
                        .fill_q
                        .produce_and_wakeup(&dev1_frames[..pkts_rcvd], dev1.rx_q.fd(), MS_TIMEOUT)
                        .unwrap()
                        != pkts_rcvd
                    {
                        // Loop until frames added to the fill ring.
                    }

                    total_pkts_rcvd += pkts_rcvd;
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
                    total_pkts_consumed += pkts_sent;

                    if total_pkts_sent < NUM_PACKETS_TO_SEND {
                        // Data is still contained in the frames so just set the descriptor's length
                        for desc in dev2_frames[..pkts_sent].iter_mut() {
                            desc.set_len(msg_size);
                        }

                        // Wait until we're ok to write
                        while !socket::poll_write(dev2.tx_q.fd(), MS_TIMEOUT).unwrap() {
                            continue;
                        }

                        let pkts_to_send = cmp::min(
                            MAX_BATCH_SIZE,
                            cmp::min(pkts_sent, NUM_PACKETS_TO_SEND - total_pkts_sent),
                        );

                        // Add consumed frames back to the tx queue
                        while dev2
                            .tx_q
                            .produce_and_wakeup(&dev2_frames[..pkts_to_send])
                            .unwrap()
                            != pkts_to_send
                        {
                            // Loop until frames added to the tx ring.
                        }

                        total_pkts_sent += pkts_to_send;
                    }
                }
            }
        }
    }

    let elapsed_secs = start.elapsed().as_secs_f64();

    // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
    let bytes_sent_per_sec: f64 = (total_pkts_sent as f64) * (msg_size as f64) / elapsed_secs;
    let bytes_rcvd_per_sec: f64 = (total_pkts_rcvd as f64) * (msg_size as f64) / elapsed_secs;

    // 1 bit/second = 1e-9 Gbps
    // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
    let gbps_sent = bytes_sent_per_sec / 0.125e9;
    let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

    println!(
        "time taken to send {} {}-byte messages: {:.3} secs",
        NUM_PACKETS_TO_SEND, msg_size, elapsed_secs
    );
    println!(
        "send throughput: {:.3} Gbps (msgs sent: {})",
        gbps_sent, total_pkts_sent
    );
    println!(
        "recv throughout: {:.3} Gbps (msgs rcvd: {})",
        gbps_rcvd, total_pkts_rcvd
    );
}

fn dev2_to_dev1_multithreaded(
    mut dev1: SocketState<'static>,
    mut dev2: SocketState<'static>,
    msg_size: u32,
) {
    let (d1_to_d2_tx, d1_to_d2_rx): (Sender<()>, Receiver<()>) = mpsc::channel();

    let start = Instant::now();

    // Spawn the receiver thread
    let rx_handle = thread::spawn(move || {
        let dev1_frames = &mut dev1.frame_descs;
        let fill_q_size: usize = FILL_Q_SIZE.try_into().unwrap();

        // Populate fill queue
        let frames_filled = dev1
            .fill_q
            .produce_and_wakeup(&dev1_frames[..fill_q_size], dev1.rx_q.fd(), MS_TIMEOUT)
            .unwrap();

        assert_eq!(frames_filled, fill_q_size);

        if let Err(_) = d1_to_d2_tx.send(()) {
            println!("sender thread has gone away");
            return 0;
        }

        let mut total_pkts_rcvd = 0;
        let mut pkts_rcvd_this_batch = 0;

        let fill_q_size: usize = FILL_Q_SIZE.try_into().unwrap();
        let fill_q_refill_thresh = fill_q_size / 3;

        while total_pkts_rcvd < NUM_PACKETS_TO_SEND {
            match dev1
                .rx_q
                .poll_and_consume(&mut dev1_frames[..], MS_TIMEOUT)
                .unwrap()
            {
                0 => {
                    // No packets consumed, wake up fill queue if required
                    if dev1.fill_q.needs_wakeup() {
                        dev1.fill_q.wakeup(dev1.rx_q.fd(), MS_TIMEOUT).unwrap();
                    }

                    // Or it might be that there are no packets left to receive
                    if SENDER_DONE.load(Ordering::SeqCst) {
                        return total_pkts_rcvd;
                    }
                }
                npkts => {
                    total_pkts_rcvd += npkts;
                    pkts_rcvd_this_batch += npkts;

                    if pkts_rcvd_this_batch > fill_q_refill_thresh {
                        let nframes_to_produce = cmp::min(fill_q_size, pkts_rcvd_this_batch);

                        // Add frames back to fill queue
                        while dev1
                            .fill_q
                            .produce_and_wakeup(
                                &dev1_frames[..nframes_to_produce],
                                dev1.rx_q.fd(),
                                MS_TIMEOUT,
                            )
                            .unwrap()
                            != nframes_to_produce
                        {
                            // Loop until frames added to the fill ring.
                        }

                        pkts_rcvd_this_batch -= nframes_to_produce;
                    }
                }
            }
        }

        total_pkts_rcvd
    });

    // Spawn the sender thread
    let tx_handle = thread::spawn(move || {
        let dev2_frames = &mut dev2.frame_descs;

        // Populate tx queue.
        let mut total_pkts_sent = 0;
        let mut total_pkts_submitted = dev2.tx_q.produce(&dev2_frames[..MAX_BATCH_SIZE]);

        assert_eq!(total_pkts_submitted, MAX_BATCH_SIZE);

        // Let the receiver populate its fill queue first and wait for the go-ahead.
        if let Err(_) = d1_to_d2_rx.recv() {
            println!("receiver thread has gone away");
            return 0;
        }

        while total_pkts_sent < NUM_PACKETS_TO_SEND {
            // Let sent packets catch up with submitted packets
            let mut pkts_sent = 0;

            while pkts_sent < total_pkts_submitted - total_pkts_sent {
                match dev2.comp_q.consume(&mut dev2_frames[pkts_sent..]) {
                    0 => {
                        // In copy mode so need to make a syscall to actually send packets
                        // despite having produced frames onto the fill ring.
                        if dev2.tx_q.needs_wakeup() {
                            dev2.tx_q.wakeup().unwrap();
                        }
                    }
                    npkts => {
                        pkts_sent += npkts;
                    }
                }
            }

            total_pkts_sent += pkts_sent;

            if total_pkts_submitted < NUM_PACKETS_TO_SEND {
                // Data is still contained in the frames so just set the descriptor's length
                for desc in dev2_frames[..pkts_sent].iter_mut() {
                    desc.set_len(msg_size);
                }

                // Wait until we're ok to write
                while !socket::poll_write(dev2.tx_q.fd(), MS_TIMEOUT).unwrap() {
                    continue;
                }

                let pkts_to_submit =
                    cmp::min(pkts_sent, NUM_PACKETS_TO_SEND - total_pkts_submitted);

                let pkts_to_submit = cmp::min(pkts_to_submit, MAX_BATCH_SIZE);

                // Add consumed frames back to the tx queue
                while dev2
                    .tx_q
                    .produce_and_wakeup(&dev2_frames[..pkts_to_submit])
                    .unwrap()
                    != pkts_to_submit
                {
                    // Loop until frames added to the tx ring.
                }

                total_pkts_submitted += pkts_to_submit;
            }
        }

        // Mark sender as done so receiver knows when to return
        SENDER_DONE.store(true, Ordering::SeqCst);

        total_pkts_sent
    });

    let tx_res = tx_handle.join();
    let rx_res = rx_handle.join();

    if let (Ok(pkts_sent), Ok(pkts_rcvd)) = (&tx_res, &rx_res) {
        let elapsed_secs = start.elapsed().as_secs_f64();

        // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
        let bytes_sent_per_sec: f64 = (*pkts_sent as f64) * (msg_size as f64) / elapsed_secs;
        let bytes_rcvd_per_sec: f64 = (*pkts_rcvd as f64) * (msg_size as f64) / elapsed_secs;

        // 1 bit/second = 1e-9 Gbps
        // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
        let gbps_sent = bytes_sent_per_sec / 0.125e9;
        let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

        println!(
            "time taken to send {} {}-byte messages: {:.3} secs",
            NUM_PACKETS_TO_SEND, msg_size, elapsed_secs
        );
        println!(
            "send throughput: {:.3} Gbps (msgs sent: {})",
            gbps_sent, pkts_sent
        );
        println!(
            "recv throughout: {:.3} Gbps (msgs rcvd: {})",
            gbps_rcvd, pkts_rcvd
        );
    } else {
        println!("error (tx_res: {:?}) (rx_res: {:?})", tx_res, rx_res);
    }
}

fn run_example(veth_config: &VethConfig, use_multithreaded: bool) {
    // Create umem and socket configs
    let umem_config = UmemConfig::new(
        NonZeroU32::new(FRAME_COUNT).unwrap(),
        NonZeroU32::new(FRAME_SIZE).unwrap(),
        FILL_Q_SIZE,
        COMP_Q_SIZE,
        0,
        false,
    )
    .unwrap();

    let socket_config = SocketConfig::new(
        RX_Q_SIZE,
        TX_Q_SIZE,
        LibbpfFlags::empty(),
        XdpFlags::empty(),
        BindFlags::XDP_USE_NEED_WAKEUP,
    )
    .unwrap();

    let dev1 = build_socket_and_umem(
        umem_config.clone(),
        socket_config.clone(),
        veth_config.dev1_name(),
        0,
    );

    let mut dev2 = build_socket_and_umem(umem_config, socket_config, veth_config.dev2_name(), 0);

    // Copy over some bytes to dev2s umem to transmit
    let eth_frame = generate_eth_frame(veth_config, PAYLOAD_SIZE);

    for desc in dev2.frame_descs.iter_mut() {
        dev2.umem.copy_data_to_frame(desc, &eth_frame[..]).unwrap();

        assert_eq!(desc.len(), eth_frame.len().try_into().unwrap());
    }

    // Send messages
    if use_multithreaded {
        println!(
            "sending {} eth frames w/ {}-byte payload (total msg size: {} bytes) (multi-threaded)",
            NUM_PACKETS_TO_SEND,
            PAYLOAD_SIZE,
            eth_frame.len()
        );
        dev2_to_dev1_multithreaded(dev1, dev2, eth_frame.len().try_into().unwrap());
    } else {
        println!(
            "sending {} eth frames w/ {}-byte payload (total msg size: {} bytes) (single-threaded)",
            NUM_PACKETS_TO_SEND,
            PAYLOAD_SIZE,
            eth_frame.len()
        );
        dev2_to_dev1_single_thread(dev1, dev2, eth_frame.len().try_into().unwrap());
    }
}

fn main() {
    let matches = App::new("dev1_to_dev2")
        .arg(
            Arg::with_name("multithreaded")
                .short("m")
                .long("multithreaded")
                .required(false)
                .takes_value(false),
        )
        .get_matches();

    let use_multithreaded = matches.is_present("multithreaded");

    let veth_config = VethConfig::new(
        String::from("xsk_ex_dev1"),
        String::from("xsk_ex_dev2"),
        [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a],
        [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31],
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 2), 24),
    );

    let veth_config_clone = veth_config.clone();

    let (startup_w, mut startup_r) = oneshot::channel();
    let (shutdown_w, shutdown_r) = oneshot::channel();

    // Create the veth link
    let veth_handle = thread::spawn(move || {
        let mut runtime = Runtime::new().unwrap();

        runtime.block_on(setup::run_veth_link(
            &veth_config_clone,
            startup_w,
            shutdown_r,
        ))
    });

    loop {
        match startup_r.try_recv() {
            Ok(_) => break,
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Closed) => panic!("Failed to set up veth link"),
        }
    }

    println!("veth setup complete, press enter to continue");

    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);

    // Run example in separate thread so that if it panics we can clean up here
    let ex_handle = thread::spawn(move || run_example(&veth_config, use_multithreaded));

    let res = ex_handle.join();

    // Tell link to close
    if let Err(e) = shutdown_w.send(()) {
        eprintln!("veth link thread returned unexpectedly: {:?}", e);
    }

    veth_handle.join().unwrap();

    res.unwrap();
}
