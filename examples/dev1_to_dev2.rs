use crossbeam_channel::{self, Receiver, Sender};
use std::{
    cmp,
    convert::TryInto,
    fmt::Debug,
    io::Write,
    iter,
    net::Ipv4Addr,
    num::NonZeroU32,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Instant,
};
use structopt::StructOpt;
use tokio::runtime::Runtime;
use xsk_rs::{
    config::{BindFlags, FrameSize, Interface, QueueSize, SocketConfig, UmemConfig},
    CompQueue, FillQueue, FrameDesc, RxQueue, Socket, TxQueue, Umem,
};

mod setup;
use setup::{util, veth_setup, LinkIpAddr, PacketGenerator, VethDevConfig};

// Reqd for the multithreaded case to signal when all packets have
// been sent
static SENDER_DONE: AtomicBool = AtomicBool::new(false);

pub struct Xsk {
    pub umem: Umem,
    pub fq: FillQueue,
    pub cq: CompQueue,
    pub tx_q: TxQueue,
    pub rx_q: RxQueue,
    pub descs: Vec<FrameDesc>,
}

#[derive(Debug, Clone, Copy)]
struct XskConfig {
    tx_q_size: QueueSize,
    rx_q_size: QueueSize,
    cq_size: QueueSize,
    fq_size: QueueSize,
    frame_size: FrameSize,
    frame_count: u32,
}

#[derive(Debug, Clone, Copy)]
struct Config {
    multithreaded: bool,
    poll_ms_timeout: i32,
    payload_size: usize,
    max_batch_size: usize,
    num_packets_to_send: usize,
    sender: XskConfig,
    receiver: XskConfig,
}

impl From<Opt> for Config {
    fn from(opt: Opt) -> Self {
        let sender = XskConfig {
            tx_q_size: opt.tx_q_size_sender.try_into().unwrap(),
            rx_q_size: opt.rx_q_size_sender.try_into().unwrap(),
            cq_size: opt.cq_size_sender.try_into().unwrap(),
            fq_size: opt.fq_size_sender.try_into().unwrap(),
            frame_count: opt.fq_size_sender + opt.cq_size_sender,
            frame_size: opt.frame_size_sender.try_into().unwrap(),
        };

        let receiver = XskConfig {
            tx_q_size: opt.tx_q_size_receiver.try_into().unwrap(),
            rx_q_size: opt.rx_q_size_receiver.try_into().unwrap(),
            cq_size: opt.cq_size_receiver.try_into().unwrap(),
            fq_size: opt.fq_size_receiver.try_into().unwrap(),
            frame_count: opt.fq_size_receiver + opt.cq_size_receiver,
            frame_size: opt.frame_size_receiver.try_into().unwrap(),
        };

        Config {
            multithreaded: opt.multithreaded,
            poll_ms_timeout: opt.poll_ms_timeout,
            payload_size: opt.payload_size,
            max_batch_size: opt.max_batch_size,
            num_packets_to_send: opt.num_packets_to_send,
            sender,
            receiver,
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "dev1_to_dev2")]
struct Opt {
    /// Run sender and receiver in separate threads
    #[structopt(short, long)]
    multithreaded: bool,

    /// Sender fill queue size
    #[structopt(default_value = "8192")]
    fq_size_sender: u32,

    /// Sender comp queue size
    #[structopt(default_value = "4096")]
    cq_size_sender: u32,

    /// Sender tx queue size
    #[structopt(default_value = "4096")]
    tx_q_size_sender: u32,

    /// Sender rx queue size
    #[structopt(default_value = "4096")]
    rx_q_size_sender: u32,

    /// Sender frame size
    #[structopt(default_value = "2048")]
    frame_size_sender: u32,

    /// Receiver fill queue size
    #[structopt(default_value = "8192")]
    fq_size_receiver: u32,

    /// Receiver comp queue size
    #[structopt(default_value = "4096")]
    cq_size_receiver: u32,

    /// Receiver tx queue size
    #[structopt(default_value = "4096")]
    tx_q_size_receiver: u32,

    /// Receuver rx queue size
    #[structopt(default_value = "4096")]
    rx_q_size_receiver: u32,

    /// Receiver frame size
    #[structopt(default_value = "2048")]
    frame_size_receiver: u32,

    /// Socket poll timeout in milliseconds
    #[structopt(default_value = "100")]
    poll_ms_timeout: i32,

    /// Packet payload size
    #[structopt(default_value = "32")]
    payload_size: usize,

    /// Max number of packets to send at once
    #[structopt(default_value = "64")]
    max_batch_size: usize,

    /// Total number of packets to send
    #[structopt(default_value = "5000000")]
    num_packets_to_send: usize,
}

fn dev1_to_dev2_single_thread(
    config: Config,
    tx: (Xsk, PacketGenerator),
    rx: (Xsk, PacketGenerator),
) {
    let (mut xsk_tx, pkt_gen) = tx;
    let (mut xsk_rx, _) = rx;

    let rx_cfg = config.receiver;

    let tx_umem = &xsk_tx.umem;

    let tx_descs = &mut xsk_tx.descs;
    let rx_descs = &mut xsk_rx.descs;

    let start = Instant::now();

    // Packets to write
    let mut pkts = iter::repeat_with(|| {
        pkt_gen
            .generate_packet(1234, 1234, config.payload_size)
            .unwrap()
    });

    // Populate receiver fill queue
    let frames_filled = unsafe {
        xsk_rx
            .fq
            .produce(&rx_descs[..rx_cfg.fq_size.get() as usize])
    };

    assert_eq!(frames_filled, rx_cfg.fq_size.get() as usize);

    log::debug!("frames added to receiver fill queue: {}", frames_filled);

    // Write packets to UMEM and populate sender tx queue
    tx_descs[0..config.max_batch_size]
        .iter_mut()
        .for_each(|desc| {
            let pkt = pkts.next().unwrap();

            unsafe {
                tx_umem.data_mut(desc).cursor().write_all(&pkt).unwrap();
            }
        });

    let mut total_frames_sent = unsafe { xsk_tx.tx_q.produce(&tx_descs[..config.max_batch_size]) };

    assert_eq!(total_frames_sent, config.max_batch_size);

    log::debug!("frames added to sender tx queue: {}", total_frames_sent);

    let mut total_frames_rcvd = 0;
    let mut total_frames_consumed = 0;

    while total_frames_consumed < config.num_packets_to_send
        || total_frames_rcvd < config.num_packets_to_send
    {
        while total_frames_rcvd < total_frames_sent {
            // In copy mode tx is driven by a syscall, so we need to
            // wakeup the kernel with a call to either sendto() or
            // poll() (wakeup() below uses sendto()).
            if xsk_tx.tx_q.needs_wakeup() {
                log::debug!("waking up sender tx queue");
                xsk_tx.tx_q.wakeup().unwrap();
            }

            // Handle rx
            match unsafe {
                xsk_rx
                    .rx_q
                    .poll_and_consume(&mut tx_descs[..], config.poll_ms_timeout)
                    .unwrap()
            } {
                0 => {
                    // No frames consumed, wake up fill queue if required
                    log::debug!("receiver rx queue consumed 0 frames");

                    if xsk_rx.fq.needs_wakeup() {
                        log::debug!("waking up receiver fill queue");
                        let fd = xsk_rx.rx_q.fd_mut();
                        xsk_rx.fq.wakeup(fd, config.poll_ms_timeout).unwrap();
                    }
                }
                frames_rcvd => {
                    log::debug!("receiver rx queue consumed {} frames", frames_rcvd);

                    // Add frames back to fill queue
                    while unsafe {
                        let fd = xsk_rx.rx_q.fd_mut();
                        xsk_rx
                            .fq
                            .produce_and_wakeup(
                                &rx_descs[..frames_rcvd],
                                fd,
                                config.poll_ms_timeout,
                            )
                            .unwrap()
                    } != frames_rcvd
                    {
                        // Loop until frames added to the fill ring.
                        log::debug!("receiver fill queue failed to allocate");
                    }

                    log::debug!("submitted {} frames to receiver fill queue", frames_rcvd);

                    total_frames_rcvd += frames_rcvd;

                    log::debug!("total frames received: {}", total_frames_rcvd);
                }
            }
        }

        if total_frames_sent < config.num_packets_to_send
            || total_frames_consumed < config.num_packets_to_send
        {
            // Handle tx
            match unsafe { xsk_tx.cq.consume(&mut tx_descs[..]) } {
                0 => {
                    log::debug!("sender comp queue consumed 0 frames");

                    if xsk_tx.tx_q.needs_wakeup() {
                        log::debug!("waking up sender tx queue");
                        xsk_tx.tx_q.wakeup().unwrap();
                    }
                }
                frames_rcvd => {
                    log::debug!("sender comp queue consumed {} frames", frames_rcvd);

                    total_frames_consumed += frames_rcvd;

                    if total_frames_sent < config.num_packets_to_send {
                        // Write new data
                        tx_descs[..frames_rcvd].iter_mut().for_each(|desc| {
                            let pkt = pkts.next().unwrap();

                            unsafe {
                                tx_umem.data_mut(desc).cursor().write_all(&pkt).unwrap();
                            }
                        });

                        // Wait until we're ok to write
                        while !xsk_tx.tx_q.poll(config.poll_ms_timeout).unwrap() {
                            log::debug!("sender socket not ready to write");
                            continue;
                        }

                        let frames_to_send = cmp::min(
                            frames_rcvd,
                            cmp::min(
                                config.max_batch_size,
                                config.num_packets_to_send - total_frames_sent,
                            ),
                        );

                        // Add consumed frames back to the tx queue
                        while unsafe {
                            xsk_tx
                                .tx_q
                                .produce_and_wakeup(&tx_descs[..frames_to_send])
                                .unwrap()
                        } != frames_to_send
                        {
                            // Loop until frames added to the tx ring.
                            log::debug!("sender tx queue failed to allocate");
                        }
                        log::debug!("submitted {} frames to sender tx queue", frames_to_send);

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
    let pkt_len = pkts.next().unwrap().len();

    let bytes_sent_per_sec: f64 = (total_frames_sent as f64) * (pkt_len as f64) / elapsed_secs;
    let bytes_rcvd_per_sec: f64 = (total_frames_rcvd as f64) * (pkt_len as f64) / elapsed_secs;

    // 1 bit/second = 1e-9 Gbps
    // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
    let gbps_sent = bytes_sent_per_sec / 0.125e9;
    let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

    // Note that this is being
    println!(
        "time taken to send {} {}-byte eth frames: {:.3} secs",
        config.num_packets_to_send, pkt_len, elapsed_secs
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

fn dev1_to_dev2_multithreaded(
    config: Config,
    tx: (Xsk, PacketGenerator),
    rx: (Xsk, PacketGenerator),
) {
    let rx_cfg = config.receiver;

    let payload_size = config.payload_size;
    let max_batch_size = config.max_batch_size;
    let num_frames_to_send = config.num_packets_to_send;
    let poll_ms_timeout = config.poll_ms_timeout;

    let (begin_send_tx, begin_send_rx): (Sender<()>, Receiver<()>) = crossbeam_channel::bounded(1);

    let start = Instant::now();

    let (mut xsk_tx, pkt_gen) = tx;

    // Packets to write
    let mut pkts =
        iter::repeat_with(move || pkt_gen.generate_packet(1234, 1234, payload_size).unwrap());

    let pkt_len = pkts.next().unwrap().len();

    let rx_handle = thread::spawn(move || {
        let (mut xsk_rx, _) = rx;

        let rx_frames = &mut xsk_rx.descs;

        // Populate receiver fill queue
        let frames_filled = unsafe {
            xsk_rx
                .fq
                .produce(&rx_frames[..rx_cfg.fq_size.get() as usize])
        };

        assert_eq!(frames_filled, rx_cfg.fq_size.get() as usize);

        log::debug!("frames added to receiver fill queue: {}", frames_filled);

        if let Err(_) = begin_send_tx.send(()) {
            println!("sender thread has gone away");
            return 0;
        }

        let mut total_frames_rcvd = 0;

        while total_frames_rcvd < num_frames_to_send {
            // Handle rx
            match unsafe {
                xsk_rx
                    .rx_q
                    .poll_and_consume(&mut rx_frames[..], poll_ms_timeout)
                    .unwrap()
            } {
                0 => {
                    // No frames consumed, wake up fill queue if required
                    log::debug!("receiver rx queue consumed 0 frames");

                    if xsk_rx.fq.needs_wakeup() {
                        log::debug!("waking up receiver fill queue");
                        let fd = xsk_rx.rx_q.fd_mut();
                        xsk_rx.fq.wakeup(fd, poll_ms_timeout).unwrap();
                    }

                    // Or it might be that there are no packets left to receive
                    if SENDER_DONE.load(Ordering::Relaxed) {
                        break;
                    }
                }
                frames_rcvd => {
                    log::debug!("receiver rx queue consumed {} frames", frames_rcvd);

                    // Add frames back to fill queue
                    while unsafe {
                        let fd = xsk_rx.rx_q.fd_mut();
                        xsk_rx
                            .fq
                            .produce_and_wakeup(&rx_frames[..frames_rcvd], fd, poll_ms_timeout)
                            .unwrap()
                    } != frames_rcvd
                    {
                        // Loop until frames added to the fill ring.
                        log::debug!("receiver fill queue failed to allocate");
                    }

                    log::debug!("submitted {} frames to receiver fill queue", frames_rcvd);

                    total_frames_rcvd += frames_rcvd;

                    log::debug!("total frames received: {}", total_frames_rcvd);
                }
            }
        }

        log::debug!("receiver complete");

        total_frames_rcvd
    });

    let tx_handle = thread::spawn(move || {
        let tx_umem = &xsk_tx.umem;
        let tx_descs = &mut xsk_tx.descs;

        tx_descs[0..max_batch_size].iter_mut().for_each(|frame| {
            let pkt = pkts.next().unwrap();

            unsafe {
                tx_umem.data_mut(frame).cursor().write_all(&pkt).unwrap();
            }
        });

        let mut total_frames_consumed = 0;

        let mut total_frames_sent = unsafe { xsk_tx.tx_q.produce(&tx_descs[..max_batch_size]) };

        assert_eq!(total_frames_sent, max_batch_size);

        log::debug!("frames added to sender tx queue: {}", total_frames_sent);

        // Let the receiver populate its fill queue first and wait for the go-ahead.
        if let Err(_) = begin_send_rx.recv() {
            println!("receiver thread has gone away");
            return 0;
        }

        while total_frames_consumed < num_frames_to_send {
            match unsafe { xsk_tx.cq.consume(&mut tx_descs[..]) } {
                0 => {
                    log::debug!("sender comp queue consumed 0 frames");

                    if xsk_tx.tx_q.needs_wakeup() {
                        log::debug!("waking up sender tx queue");
                        xsk_tx.tx_q.wakeup().unwrap();
                    }
                }
                frames_rcvd => {
                    log::debug!("sender comp queue consumed {} frames", frames_rcvd);

                    total_frames_consumed += frames_rcvd;

                    if total_frames_sent < num_frames_to_send {
                        // Write new data
                        tx_descs[..frames_rcvd].iter_mut().for_each(|desc| {
                            let pkt = pkts.next().unwrap();

                            unsafe {
                                tx_umem.data_mut(desc).cursor().write_all(&pkt).unwrap();
                            }
                        });

                        // Wait until we're ok to write
                        while !xsk_tx.tx_q.poll(poll_ms_timeout).unwrap() {
                            log::debug!("sender socket not ready to write");
                            continue;
                        }

                        let frames_to_send = cmp::min(
                            frames_rcvd,
                            cmp::min(max_batch_size, num_frames_to_send - total_frames_sent),
                        );

                        // Add consumed frames back to the tx queue
                        while unsafe {
                            xsk_tx
                                .tx_q
                                .produce_and_wakeup(&tx_descs[..frames_to_send])
                                .unwrap()
                        } != frames_to_send
                        {
                            // Loop until frames added to the tx ring.
                            log::debug!("sender tx queue failed to allocate");
                        }
                        log::debug!("submitted {} frames to sender tx queue", frames_to_send);

                        total_frames_sent += frames_to_send;
                    }

                    log::debug!("total frames consumed: {}", total_frames_consumed);
                    log::debug!("total frames sent: {}", total_frames_sent);
                }
            }
        }

        log::debug!("sender complete");

        // Mark sender as done so receiver knows when to return
        SENDER_DONE.store(true, Ordering::Relaxed);

        total_frames_consumed
    });

    let tx_res = tx_handle.join();
    let rx_res = rx_handle.join();

    if let (Ok(pkts_sent), Ok(pkts_rcvd)) = (&tx_res, &rx_res) {
        let elapsed_secs = start.elapsed().as_secs_f64();

        // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
        let bytes_sent_per_sec: f64 = (*pkts_sent as f64) * (pkt_len as f64) / elapsed_secs;
        let bytes_rcvd_per_sec: f64 = (*pkts_rcvd as f64) * (pkt_len as f64) / elapsed_secs;

        // 1 bit/second = 1e-9 Gbps
        // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
        let gbps_sent = bytes_sent_per_sec / 0.125e9;
        let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

        println!(
            "time taken to send {} {}-byte eth frames: {:.3} secs",
            config.num_packets_to_send, pkt_len, elapsed_secs
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

pub fn build_socket_and_umem(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    frame_count: NonZeroU32,
    if_name: &Interface,
    queue_id: u32,
) -> Xsk {
    let (umem, frames) = Umem::new(umem_config, frame_count, false).expect("failed to build umem");

    let (tx_q, rx_q, fq_and_cq) = unsafe {
        Socket::new(socket_config, &umem, if_name, queue_id).expect("failed to build socket")
    };

    let (fq, cq) = fq_and_cq.expect(&format!(
        "missing fill and comp queue - interface {:?} may already be bound to",
        if_name
    ));

    Xsk {
        umem,
        fq,
        cq,
        tx_q,
        rx_q,
        descs: frames,
    }
}

fn build_umem_and_socket_config(config: &XskConfig) -> (UmemConfig, SocketConfig) {
    let umem_config = UmemConfig::builder()
        .frame_size(config.frame_size)
        .fill_queue_size(config.fq_size)
        .comp_queue_size(config.cq_size)
        .build()
        .unwrap();

    let socket_config = SocketConfig::builder()
        .rx_queue_size(config.rx_q_size)
        .tx_queue_size(config.tx_q_size)
        .bind_flags(BindFlags::XDP_USE_NEED_WAKEUP)
        .build();

    (umem_config, socket_config)
}

fn run_example(
    config: Config,
    dev_tx: (VethDevConfig, PacketGenerator),
    dev_rx: (VethDevConfig, PacketGenerator),
) {
    let (umem_config_tx, socket_config_tx) = build_umem_and_socket_config(&config.sender);
    let (umem_config_rx, socket_config_rx) = build_umem_and_socket_config(&config.receiver);

    let xsk_tx = build_socket_and_umem(
        umem_config_tx.clone(),
        socket_config_tx.clone(),
        config.sender.frame_count.try_into().unwrap(),
        &dev_tx.0.if_name().parse().unwrap(),
        0,
    );

    let xsk_rx = build_socket_and_umem(
        umem_config_rx.clone(),
        socket_config_rx.clone(),
        config.receiver.frame_count.try_into().unwrap(),
        &dev_rx.0.if_name().parse().unwrap(),
        0,
    );

    if config.multithreaded {
        println!(
            "sending {} eth frames w/ {}-byte payload (total msg size: {} bytes) (multi-threaded)",
            config.num_packets_to_send,
            config.payload_size,
            &config.sender.frame_size.get()
        );
        dev1_to_dev2_multithreaded(config, (xsk_tx, dev_tx.1), (xsk_rx, dev_rx.1));
    } else {
        println!(
            "sending {} eth frames w/ {}-byte payload (total msg size: {} bytes) (single-threaded)",
            config.num_packets_to_send,
            config.payload_size,
            &config.sender.frame_size.get()
        );
        dev1_to_dev2_single_thread(config, (xsk_tx, dev_tx.1), (xsk_rx, dev_rx.1));
    }
}

fn main() {
    env_logger::init();

    let config = Opt::from_args().into();

    let dev1_config = VethDevConfig {
        if_name: "xsk_test_dev1".into(),
        addr: [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a],
        ip_addr: LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
    };

    let dev2_config = VethDevConfig {
        if_name: "xsk_test_dev2".into(),
        addr: [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31],
        ip_addr: LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
    };

    // We'll keep track of ctrl+c events but not let them kill the process
    // immediately as we may need to clean up the veth pair.
    let ctrl_c_events = util::ctrl_channel().unwrap();

    let (complete_tx, complete_rx) = crossbeam_channel::bounded(1);

    let runtime = Runtime::new().unwrap();

    let example_handle = thread::spawn(move || {
        let res = runtime.block_on(veth_setup::run_with_veth_pair(
            dev1_config,
            dev2_config,
            move |dev1, dev2| run_example(config, dev1, dev2),
        ));

        let _ = complete_tx.send(());

        res
    });

    // Wait for either the example to finish or for a ctrl+c event to occur
    crossbeam_channel::select! {
        recv(complete_rx) -> _ => {
        },
        recv(ctrl_c_events) -> _ => {
            println!("SIGINT received");
        }
    }

    example_handle.join().unwrap().unwrap();
}
