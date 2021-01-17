mod setup;

use clap::{App, Arg};
use crossbeam_channel::{self, Receiver, Sender};
use std::{
    cmp,
    fmt::Debug,
    net::Ipv4Addr,
    num::NonZeroU32,
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
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

// Reqd for the multithreaded case to signal when all packets have
// been sent
static SENDER_DONE: AtomicBool = AtomicBool::new(false);

// Umem has to go last to maintain correct drop order!  If, for
// example, we tried to drop the umem before the queues then the
// destructor would fail as the memory is still in use.
struct Xsk<'umem> {
    tx_q: TxQueue<'umem>,
    rx_q: RxQueue<'umem>,
    fill_q: FillQueue<'umem>,
    comp_q: CompQueue<'umem>,
    frame_descs: Vec<FrameDesc<'umem>>,
    umem: Umem<'umem>,
}

#[derive(Clone, Debug)]
struct Config {
    is_multithreaded: bool,
    tx_q_size: u32,
    rx_q_size: u32,
    comp_q_size: u32,
    fill_q_size: u32,
    frame_size: u32,
    poll_ms_timeout: i32,
    payload_size: usize,
    max_batch_size: usize,
    num_frames_to_send: usize,
}

/// Send ETH frames with payload size `msg_size` through dev2 to be received by dev1.
/// This is single threaded so will handle the send and receive process alternately.
fn dev2_to_dev1_single_thread(config: &Config, mut dev1: Xsk, mut dev2: Xsk) {
    // Extra 42 bytes is for the eth and IP headers
    let sent_eth_frame_size = config.payload_size + 42;

    let dev1_frames = &mut dev1.frame_descs;
    let dev2_frames = &mut dev2.frame_descs;

    let start = Instant::now();

    // Populate fill queue
    let frames_filled = unsafe {
        dev1.fill_q
            .produce(&dev1_frames[..config.fill_q_size as usize])
    };

    assert_eq!(frames_filled, config.fill_q_size as usize);
    log::debug!("init frames added to dev1.fill_q: {}", frames_filled);

    // Populate tx queue
    let mut total_frames_sent = unsafe { dev2.tx_q.produce(&dev2_frames[..config.max_batch_size]) };

    assert_eq!(total_frames_sent, config.max_batch_size);
    log::debug!("init frames added to dev2.tx_q: {}", total_frames_sent);

    let mut total_frames_rcvd = 0;
    let mut total_frames_consumed = 0;

    while total_frames_consumed < config.num_frames_to_send
        || total_frames_rcvd < config.num_frames_to_send
    {
        while total_frames_rcvd < total_frames_sent {
            // In copy mode tx is driven by a syscall, so we need to
            // wakeup the kernel with a call to either sendto() or
            // poll() (wakeup() below uses sendto()).
            if dev2.tx_q.needs_wakeup() {
                log::debug!("waking up dev2.tx_q");
                dev2.tx_q.wakeup().unwrap();
            }

            // Handle rx
            match dev1
                .rx_q
                .poll_and_consume(&mut dev1_frames[..], config.poll_ms_timeout)
                .unwrap()
            {
                0 => {
                    // No frames consumed, wake up fill queue if required
                    log::debug!("dev1.rx_q.poll_and_consume() consumed 0 frames");
                    if dev1.fill_q.needs_wakeup() {
                        log::debug!("waking up dev1.fill_q");
                        dev1.fill_q
                            .wakeup(dev1.rx_q.fd(), config.poll_ms_timeout)
                            .unwrap();
                    }
                }
                frames_rcvd => {
                    log::debug!(
                        "dev1.rx_q.poll_and_consume() consumed {} frames",
                        frames_rcvd
                    );
                    // Add frames back to fill queue
                    while unsafe {
                        dev1.fill_q
                            .produce_and_wakeup(
                                &dev1_frames[..frames_rcvd],
                                dev1.rx_q.fd(),
                                config.poll_ms_timeout,
                            )
                            .unwrap()
                    } != frames_rcvd
                    {
                        // Loop until frames added to the fill ring.
                        log::debug!("dev1.fill_q.produce_and_wakeup() failed to allocate");
                    }
                    log::debug!(
                        "dev1.fill_q.produce_and_wakeup() submitted {} frames",
                        frames_rcvd
                    );

                    total_frames_rcvd += frames_rcvd;
                    log::debug!("total frames received: {}", total_frames_rcvd);
                }
            }
        }

        if total_frames_sent < config.num_frames_to_send
            || total_frames_consumed < config.num_frames_to_send
        {
            // Handle tx
            match dev2.comp_q.consume(&mut dev2_frames[..]) {
                0 => {
                    log::debug!("dev2.comp_q.consume() consumed 0 frames");
                    if dev2.tx_q.needs_wakeup() {
                        log::debug!("waking up dev2.tx_q");
                        dev2.tx_q.wakeup().unwrap();
                    }
                }
                frames_rcvd => {
                    log::debug!("dev2.comp_q.consume() consumed {} frames", frames_rcvd);
                    total_frames_consumed += frames_rcvd;

                    if total_frames_sent < config.num_frames_to_send {
                        // Data is still contained in the frames so
                        // just set the descriptor's length
                        for desc in dev2_frames[..frames_rcvd].iter_mut() {
                            desc.set_len(sent_eth_frame_size);
                        }

                        // Wait until we're ok to write
                        while !socket::poll_write(dev2.tx_q.fd(), config.poll_ms_timeout).unwrap() {
                            log::debug!("poll_write(dev2.tx_q) returned false");
                            continue;
                        }

                        let frames_to_send = cmp::min(
                            frames_rcvd,
                            cmp::min(
                                config.max_batch_size,
                                config.num_frames_to_send - total_frames_sent,
                            ),
                        );

                        // Add consumed frames back to the tx queue
                        while unsafe {
                            dev2.tx_q
                                .produce_and_wakeup(&dev2_frames[..frames_to_send])
                                .unwrap()
                        } != frames_to_send
                        {
                            // Loop until frames added to the tx ring.
                            log::debug!("dev2.tx_q.produce_and_wakeup() failed to allocate");
                        }
                        log::debug!(
                            "dev2.tx_q.produce_and_wakeup() submitted {} frames",
                            frames_to_send
                        );

                        total_frames_sent += frames_to_send;
                    }

                    log::debug!("total frames consumed: {}", total_frames_consumed);
                    log::debug!("total frames sent: {}", total_frames_sent);
                }
            }
        }
    }

    let elapsed_secs = start.elapsed().as_secs_f64();

    // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
    let bytes_sent_per_sec: f64 =
        (total_frames_sent as f64) * (sent_eth_frame_size as f64) / elapsed_secs;
    let bytes_rcvd_per_sec: f64 =
        (total_frames_rcvd as f64) * (sent_eth_frame_size as f64) / elapsed_secs;

    // 1 bit/second = 1e-9 Gbps
    // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
    let gbps_sent = bytes_sent_per_sec / 0.125e9;
    let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

    // Note that this is being
    println!(
        "time taken to send {} {}-byte eth frames: {:.3} secs",
        config.num_frames_to_send, sent_eth_frame_size, elapsed_secs
    );
    println!(
        "send throughput: {:.3} Gbps (eth frames sent: {})",
        gbps_sent, total_frames_sent
    );
    println!(
        "recv throughout: {:.3} Gbps (eth frames rcvd: {})",
        gbps_rcvd, total_frames_rcvd
    );
    println!(
        "note that these numbers are not reflective of actual AF_XDP socket performance,
since packets are being sent over a VETH pair, and so pass through the kernel"
    );
}

/// Send ETH frames with payload size `msg_size` through dev2 to be received by dev1.
/// Handle frame transmission and receipt in separate threads.
fn dev2_to_dev1_multithreaded(config: &Config, mut dev1: Xsk<'static>, mut dev2: Xsk<'static>) {
    // Extra 42 bytes is for the eth and IP headers
    let sent_eth_frame_size = config.payload_size + 42;

    // Make copies for the separate threads
    let config1 = config.clone();
    let config2 = config.clone();

    let (d1_to_d2_tx, d1_to_d2_rx): (Sender<()>, Receiver<()>) = crossbeam_channel::bounded(1);

    let start = Instant::now();

    // Spawn the receiver thread
    let rx_handle = thread::spawn(move || {
        let dev1_frames = &mut dev1.frame_descs;

        // Populate fill queue
        let frames_filled = unsafe {
            dev1.fill_q
                .produce_and_wakeup(
                    &dev1_frames[..config1.fill_q_size as usize],
                    dev1.rx_q.fd(),
                    config1.poll_ms_timeout,
                )
                .unwrap()
        };

        assert_eq!(frames_filled, config1.fill_q_size as usize);
        log::debug!("(dev1) init frames added to dev1.fill_q: {}", frames_filled);

        if let Err(_) = d1_to_d2_tx.send(()) {
            println!("sender thread (dev2) has gone away");
            return 0;
        }

        let mut total_frames_rcvd = 0;

        while total_frames_rcvd < config1.num_frames_to_send {
            match dev1
                .rx_q
                .poll_and_consume(&mut dev1_frames[..], config1.poll_ms_timeout)
                .unwrap()
            {
                0 => {
                    log::debug!("(dev1) dev1.rx_q.poll_and_consume() consumed 0 frames");
                    // No packets consumed, wake up fill queue if required
                    if dev1.fill_q.needs_wakeup() {
                        log::debug!("(dev1) waking up dev1.fill_q");
                        dev1.fill_q
                            .wakeup(dev1.rx_q.fd(), config1.poll_ms_timeout)
                            .unwrap();
                    }

                    // Or it might be that there are no packets left to receive
                    if SENDER_DONE.load(Ordering::Relaxed) {
                        break;
                    }
                }
                frames_rcvd => {
                    log::debug!(
                        "(dev1) dev1.rx_q.poll_and_consume() consumed {} frames",
                        frames_rcvd
                    );
                    // Add frames back to fill queue
                    while unsafe {
                        dev1.fill_q
                            .produce_and_wakeup(
                                &dev1_frames[..frames_rcvd],
                                dev1.rx_q.fd(),
                                config1.poll_ms_timeout,
                            )
                            .unwrap()
                    } != frames_rcvd
                    {
                        // Loop until frames added to the fill ring.
                        log::debug!("(dev1) dev1.fill_q.produce_and_wakeup() failed to allocate");
                    }

                    log::debug!(
                        "(dev1) dev1.fill_q.produce_and_wakeup() submitted {} frames",
                        frames_rcvd
                    );

                    total_frames_rcvd += frames_rcvd;
                    log::debug!("(dev1) total frames received: {}", total_frames_rcvd);
                }
            }
        }

        log::debug!("(dev1) recv complete");
        total_frames_rcvd
    });

    // Spawn the sender thread
    let tx_handle = thread::spawn(move || {
        let dev2_frames = &mut dev2.frame_descs;

        // Populate tx queue.
        let mut total_frames_consumed = 0;
        let mut total_frames_sent = unsafe {
            dev2.tx_q
                .produce_and_wakeup(&dev2_frames[..config2.max_batch_size])
                .unwrap()
        };

        assert_eq!(total_frames_sent, config2.max_batch_size);
        log::debug!(
            "(dev2) init frames added to dev2.tx_q: {}",
            total_frames_sent
        );

        // Let the receiver populate its fill queue first and wait for the go-ahead.
        if let Err(_) = d1_to_d2_rx.recv() {
            println!("receiver thread (dev1) has gone away");
            return 0;
        }

        while total_frames_consumed < config2.num_frames_to_send {
            match dev2.comp_q.consume(&mut dev2_frames[..]) {
                0 => {
                    log::debug!("(dev2) dev2.comp_q.consume() consumed 0 frames");
                    // In copy mode so need to make a syscall to actually send packets
                    // despite having produced frames onto the fill ring.
                    if dev2.tx_q.needs_wakeup() {
                        log::debug!("(dev2) waking up dev2.tx_q");
                        dev2.tx_q.wakeup().unwrap();
                    }
                }
                frames_rcvd => {
                    log::debug!(
                        "(dev2) dev2.comp_q.consume() consumed {} frames",
                        frames_rcvd
                    );

                    total_frames_consumed += frames_rcvd;

                    if total_frames_sent < config2.num_frames_to_send {
                        // Data is still contained in the frames so
                        // just set the descriptor's length
                        for desc in dev2_frames[..frames_rcvd].iter_mut() {
                            desc.set_len(sent_eth_frame_size);
                        }

                        // Wait until we're ok to write
                        while !socket::poll_write(dev2.tx_q.fd(), config2.poll_ms_timeout).unwrap()
                        {
                            log::debug!("(dev2) poll_write(dev2.tx_q) returned false");
                            continue;
                        }

                        let frames_to_send = cmp::min(
                            cmp::min(frames_rcvd, config2.max_batch_size),
                            config2.num_frames_to_send - total_frames_sent,
                        );

                        // Add consumed frames back to the tx queue
                        while unsafe {
                            dev2.tx_q
                                .produce_and_wakeup(&dev2_frames[..frames_to_send])
                                .unwrap()
                        } != frames_to_send
                        {
                            // Loop until frames added to the tx ring.
                            log::debug!("(dev2) dev2.tx_q.produce_and_wakeup() failed to allocate");
                        }

                        log::debug!(
                            "(dev2) dev2.tx_q.produce_and_wakeup() submitted {} frames",
                            frames_to_send
                        );

                        total_frames_sent += frames_to_send;
                    }

                    log::debug!("(dev2) total frames consumed: {}", total_frames_consumed);
                    log::debug!("(dev2) total frames sent: {}", total_frames_sent);
                }
            }
        }

        log::debug!("(dev2) send complete");

        // Mark sender as done so receiver knows when to return
        SENDER_DONE.store(true, Ordering::Relaxed);

        total_frames_consumed
    });

    let tx_res = tx_handle.join();
    let rx_res = rx_handle.join();

    if let (Ok(pkts_sent), Ok(pkts_rcvd)) = (&tx_res, &rx_res) {
        let elapsed_secs = start.elapsed().as_secs_f64();

        // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
        let bytes_sent_per_sec: f64 =
            (*pkts_sent as f64) * (sent_eth_frame_size as f64) / elapsed_secs;
        let bytes_rcvd_per_sec: f64 =
            (*pkts_rcvd as f64) * (sent_eth_frame_size as f64) / elapsed_secs;

        // 1 bit/second = 1e-9 Gbps
        // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
        let gbps_sent = bytes_sent_per_sec / 0.125e9;
        let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

        println!(
            "time taken to send {} {}-byte eth frames: {:.3} secs",
            config.num_frames_to_send, sent_eth_frame_size, elapsed_secs
        );
        println!(
            "send throughput: {:.3} Gbps (eth frames sent: {})",
            gbps_sent, pkts_sent
        );
        println!(
            "recv throughout: {:.3} Gbps (eth frames rcvd: {})",
            gbps_rcvd, pkts_rcvd
        );
        println!(
            "note that these numbers are not reflective of actual AF_XDP socket performance,
since packets are being sent over a VETH pair, and so pass through the kernel"
        );
    } else {
        println!("error (tx_res: {:?}) (rx_res: {:?})", tx_res, rx_res);
    }
}

fn build_socket_and_umem(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    if_name: &str,
    queue_id: u32,
) -> Xsk<'static> {
    let (mut umem, fill_q, comp_q, frame_descs) = Umem::builder(umem_config)
        .create_mmap()
        .expect(format!("failed to create mmap area for {}", if_name).as_str())
        .create_umem()
        .expect(format!("failed to create umem for {}", if_name).as_str());

    let (tx_q, rx_q) = Socket::new(socket_config, &mut umem, &if_name, queue_id)
        .expect(format!("failed to build socket for {}", if_name).as_str());

    Xsk {
        umem,
        fill_q,
        comp_q,
        tx_q,
        rx_q,
        frame_descs,
    }
}

fn run_example(config: &Config, veth_config: &VethConfig) {
    // Create umem and socket configs
    let frame_count = config.fill_q_size + config.comp_q_size;

    let umem_config = UmemConfig::new(
        NonZeroU32::new(frame_count).unwrap(),
        NonZeroU32::new(config.frame_size).unwrap(),
        config.fill_q_size,
        config.comp_q_size,
        0,
        false,
    )
    .unwrap();

    let socket_config = SocketConfig::new(
        config.rx_q_size,
        config.tx_q_size,
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
    let eth_frame = setup::generate_eth_frame(veth_config, config.payload_size);

    for desc in dev2.frame_descs.iter_mut() {
        unsafe {
            dev2.umem
                .write_to_umem_checked(desc, &eth_frame[..])
                .unwrap();
        }

        assert_eq!(desc.len(), eth_frame.len());
    }

    // Send messages
    if config.is_multithreaded {
        println!(
            "sending {} eth frames w/ {}-byte payload (total msg size: {} bytes) (multi-threaded)",
            config.num_frames_to_send,
            config.payload_size,
            eth_frame.len()
        );
        dev2_to_dev1_multithreaded(config, dev1, dev2);
    } else {
        println!(
            "sending {} eth frames w/ {}-byte payload (total msg size: {} bytes) (single-threaded)",
            config.num_frames_to_send,
            config.payload_size,
            eth_frame.len()
        );
        dev2_to_dev1_single_thread(config, dev1, dev2);
    }
}

fn parse_arg<T>(matches: &clap::ArgMatches, name: &str, err_msg: &str, default: T) -> T
where
    T: FromStr,
    <T as FromStr>::Err: Debug,
{
    matches
        .value_of(name)
        .map(|s| s.parse().expect(err_msg))
        .unwrap_or(default)
}

fn get_args() -> Config {
    let rx_q_size: u32 = 4096;
    let tx_q_size: u32 = 4096;
    let comp_q_size: u32 = 4096;
    let fill_q_size: u32 = 4096 * 4;
    let frame_size: u32 = 2048;
    let poll_ms_timeout: i32 = 100;
    let payload_size: usize = 32;
    let max_batch_size: usize = 64;
    let num_packets_to_send: usize = 5_000_000;

    let matches = App::new("dev1_to_dev2")
        .arg(
            Arg::with_name("multithreaded")
                .short("m")
                .long("multithreaded")
                .required(false)
                .takes_value(false)
                .help("Run sender and receiver in separate threads"),
        )
        .arg(
            Arg::with_name("tx_queue_size")
                .short("t")
                .long("tx-q-size")
                .required(false)
                .takes_value(true)
                .help("Set socket tx queue size"),
        )
        .arg(
            Arg::with_name("rx_queue_size")
                .short("r")
                .long("rx-q-size")
                .required(false)
                .takes_value(true)
                .help("Set socket rx queue size"),
        )
        .arg(
            Arg::with_name("comp_queue_size")
                .short("c")
                .long("comp-q-size")
                .required(false)
                .takes_value(true)
                .help("Set umem comp queue size"),
        )
        .arg(
            Arg::with_name("fill_queue_size")
                .short("f")
                .long("fill-q-size")
                .required(false)
                .takes_value(true)
                .help("Set umem fill q size"),
        )
        .arg(
            Arg::with_name("frame_size")
                .short("u")
                .long("frame-size")
                .required(false)
                .takes_value(true)
                .help("Set umem frame size"),
        )
        .arg(
            Arg::with_name("num_frames_to_send")
                .short("n")
                .long("num-frames-to-send")
                .required(false)
                .takes_value(true)
                .help("Set total number of frames to send"),
        )
        .arg(
            Arg::with_name("payload_size")
                .short("s")
                .long("payload-size")
                .required(false)
                .takes_value(true)
                .help("Set udp packet payload size"),
        )
        .arg(
            Arg::with_name("max_batch_size")
                .short("b")
                .long("max-batch-size")
                .required(false)
                .takes_value(true)
                .help("Set max number of frames possible to transmit at once"),
        )
        .arg(
            Arg::with_name("poll_ms_timeout")
                .short("p")
                .long("poll-ms-timeout")
                .required(false)
                .takes_value(true)
                .help("Set socket read/write poll timeout in milliseconds"),
        )
        .get_matches();

    let is_multithreaded = matches.is_present("multithreaded");
    let rx_q_size = parse_arg(
        &matches,
        "rx_queue_size",
        "failed to parse rx_queue_size arg",
        rx_q_size,
    );
    let tx_q_size = parse_arg(
        &matches,
        "tx_queue_size",
        "failed to parse tx_queue_size arg",
        tx_q_size,
    );
    let comp_q_size = parse_arg(
        &matches,
        "comp_queue_size",
        "failed to parse comp queue size arg",
        comp_q_size,
    );
    let fill_q_size = parse_arg(
        &matches,
        "fill_queue_size",
        "failed to parse fill queue size arg",
        fill_q_size,
    );
    let frame_size = parse_arg(
        &matches,
        "frame_size",
        "failed to parse frame size arg",
        frame_size,
    );
    let num_frames_to_send = parse_arg(
        &matches,
        "num_frames_to_send",
        "failed to parse num frames to send arg",
        num_packets_to_send,
    );
    let payload_size = parse_arg(
        &matches,
        "payload_size",
        "failed to parse payload size arg",
        payload_size,
    );
    let max_batch_size = parse_arg(
        &matches,
        "max_batch_size",
        "failed to parse max batch size arg",
        max_batch_size,
    );
    let poll_ms_timeout = parse_arg(
        &matches,
        "poll_ms_timeout",
        "failed to parse poll ms arg",
        poll_ms_timeout,
    );

    Config {
        is_multithreaded,
        tx_q_size,
        rx_q_size,
        comp_q_size,
        fill_q_size,
        frame_size,
        poll_ms_timeout,
        payload_size,
        max_batch_size,
        num_frames_to_send,
    }
}

fn main() {
    env_logger::init();

    let config = get_args();

    let veth_config = VethConfig::new(
        String::from("xsk_ex_dev1"),
        String::from("xsk_ex_dev2"),
        [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a],
        [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31],
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 2), 24),
    );

    let veth_config_clone = veth_config.clone();

    log::info!("{:#?}", config);
    log::info!("{:#?}", veth_config);

    let (startup_w, mut startup_r) = oneshot::channel();
    let (shutdown_w, shutdown_r) = oneshot::channel();

    // We'll keep track of ctrl+c events but not let them kill the
    // process immediately as we may need to clean up the veth pair.
    let ctrl_c_events = setup::ctrl_channel().unwrap();

    // Create the veth pair
    let veth_handle = thread::spawn(move || {
        let mut runtime = Runtime::new().unwrap();

        runtime.block_on(setup::run_veth_link(
            &veth_config_clone,
            startup_w,
            shutdown_r,
        ))
    });

    // Wait for confirmation that it's set up and configured
    loop {
        match startup_r.try_recv() {
            Ok(_) => break,
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Closed) => panic!("failed to set up veth pair"),
        }
    }

    // Run example in separate thread so that if it panics we can
    // clean up here
    let (example_done_tx, example_done_rx) = crossbeam_channel::bounded(1);
    let handle = thread::spawn(move || {
        run_example(&config, &veth_config);
        let _ = example_done_tx.send(());
    });

    // Wait for either the example to finish or for a ctrl+c event to occur
    crossbeam_channel::select! {
        recv(example_done_rx) -> _ => {
            // Example done
            if let Err(e) = handle.join() {
                println!("error running example: {:?}", e);
            }
        },
        recv(ctrl_c_events) -> _ => {
            println!("SIGINT received, deleting veth pair and exiting");
        }
    }

    // Delete link
    if let Err(e) = shutdown_w.send(()) {
        eprintln!("veth link thread returned unexpectedly: {:?}", e);
    }

    veth_handle.join().unwrap();
}
