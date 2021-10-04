// use crossbeam_channel::{self, Receiver, Sender};
// use std::{
//     cmp,
//     fmt::Debug,
//     iter,
//     net::Ipv4Addr,
//     sync::atomic::{AtomicBool, Ordering},
//     thread,
//     time::Instant,
// };
// use structopt::StructOpt;
// use tokio::runtime::Runtime;
// use xsk_rs::{
//     config::{BindFlags, FrameSize, Interface, QueueSize},
//     socket::fd::PollEvent,
//     CompQueue, FillQueue, FrameDesc, RxQueue, Socket, SocketConfig, TxQueue, Umem, UmemConfig,
// };

// mod setup;
// use setup::{util, veth_setup, LinkIpAddr, PacketGenerator, VethDevConfig};

// // Reqd for the multithreaded case to signal when all packets have
// // been sent
// static SENDER_DONE: AtomicBool = AtomicBool::new(false);

// pub struct Xsk {
//     pub umem: Umem,
//     pub fq: FillQueue,
//     pub cq: CompQueue,
//     pub tx_q: TxQueue,
//     pub rx_q: RxQueue,
//     pub descs: Vec<FrameDesc>,
// }

// #[derive(Clone, Debug)]
// struct XskConfig {
//     tx_q_size: QueueSize,
//     rx_q_size: QueueSize,
//     cq_size: QueueSize,
//     fq_size: QueueSize,
//     frame_size: FrameSize,
//     frame_count: u32,
// }

// #[derive(Clone, Debug)]
// struct Config {
//     is_multithreaded: bool,
//     sender: XskConfig,
//     receiver: XskConfig,
//     poll_ms_timeout: i32,
//     payload_size: usize,
//     max_batch_size: usize,
//     num_frames_to_send: usize,
// }

// impl Config {
//     fn from_opt(opt: Opt) -> Self {
//         todo!()
//     }
// }

// #[derive(Debug, StructOpt)]
// #[structopt(name = "dev1_to_dev2")]
// struct Opt {
//     /// Run sender and receiver in separate threads
//     #[structopt(short, long)]
//     multithreaded: bool,

//     /// Sender fill queue size
//     #[structopt(default_value = "8192")]
//     fq_size_sender: u32,

//     /// Sender comp queue size
//     #[structopt(default_value = "4096")]
//     cq_size_sender: u32,

//     /// Sender tx queue size
//     #[structopt(default_value = "4096")]
//     tx_q_size_sender: u32,

//     /// Sender rx queue size
//     #[structopt(default_value = "4096")]
//     rx_q_size_sender: u32,

//     /// Receiver fill queue size
//     #[structopt(default_value = "8192")]
//     fq_size_receiver: u32,

//     /// Receiver Comp queue size
//     #[structopt(default_value = "4096")]
//     cq_size_receiver: u32,

//     /// Receiver tx queue size
//     #[structopt(default_value = "4096")]
//     tx_q_size_receiver: u32,

//     /// Receuver rx queue size
//     #[structopt(default_value = "4096")]
//     rx_q_size_receiver: u32,

//     /// Socket poll timeout in milliseconds
//     #[structopt(default_value = "100")]
//     poll_ms_timeout: i32,

//     /// Packet payload size
//     #[structopt(default_value = "32")]
//     payload_size: usize,

//     /// Max number of packets to send at once
//     #[structopt(default_value = "64")]
//     max_batch_size: usize,

//     /// Total number of packets to send
//     #[structopt(default_value = "5_000_000")]
//     num_packets_to_send: usize,
// }

// /// Send ETH frames with payload size `msg_size` through dev2 to be received by dev1.
// /// This is single threaded so will handle the send and receive process alternately.
// fn dev1_to_dev2_single_thread(
//     config: &Config,
//     mut tx: (Xsk, PacketGenerator),
//     mut rx: (Xsk, PacketGenerator),
// ) {
//     let (xsk_tx, pkt_gen) = tx;
//     let (xsk_rx, _) = rx;

//     let tx_cfg = config.sender;
//     let rx_cfg = config.receiver;

//     let tx_descs = &mut xsk_tx.descs;
//     let rx_descs = &mut xsk_rx.descs;

//     let tx_fd = xsk_tx.tx_q.fd();
//     let rx_fd = xsk_rx.rx_q.fd();

//     let start = Instant::now();

//     // Packets to write
//     let pkts = iter::repeat_with(|| {
//         pkt_gen
//             .generate_packet(1234, 1234, config.payload_size)
//             .unwrap()
//     });

//     // Populate receiver fill queue
//     let frames_filled = unsafe {
//         xsk_rx
//             .fq
//             .produce(&rx_descs[..rx_cfg.fq_size.get() as usize])
//     };

//     assert_eq!(frames_filled, rx_cfg.fq_size.get() as usize);

//     log::debug!("frames added to receiver fill queue: {}", frames_filled);

//     // Write packets to UMEM and populate sender tx queue
//     tx_descs[0..config.max_batch_size]
//         .iter_mut()
//         .for_each(|desc| {
//             let pkt = pkts.next().unwrap();

//             unsafe {
//                 xsk_tx.umem.frame_data_mut(desc).write_all(&pkt).unwrap();
//             }
//         });

//     let mut total_frames_sent = unsafe { xsk_tx.tx_q.produce(&tx_descs[..config.max_batch_size]) };

//     assert_eq!(total_frames_sent, config.max_batch_size);

//     log::debug!("frames added to sender tx queue: {}", total_frames_sent);

//     let mut total_frames_rcvd = 0;
//     let mut total_frames_consumed = 0;

//     while total_frames_consumed < config.num_frames_to_send
//         || total_frames_rcvd < config.num_frames_to_send
//     {
//         while total_frames_rcvd < total_frames_sent {
//             // In copy mode tx is driven by a syscall, so we need to
//             // wakeup the kernel with a call to either sendto() or
//             // poll() (wakeup() below uses sendto()).
//             if xsk_tx.tx_q.needs_wakeup() {
//                 log::debug!("waking up sender tx queue");
//                 xsk_tx.tx_q.wakeup().unwrap();
//             }

//             // Handle rx
//             match unsafe {
//                 xsk_rx
//                     .rx_q
//                     .poll_and_consume(&mut tx_descs[..], config.poll_ms_timeout)
//                     .unwrap()
//             } {
//                 0 => {
//                     // No frames consumed, wake up fill queue if required
//                     log::debug!("receiver rx queue consumed 0 frames");

//                     if xsk_rx.fq.needs_wakeup() {
//                         log::debug!("waking up receiver fill queue");
//                         xsk_rx
//                             .fq
//                             .wakeup(&mut rx_fd, config.poll_ms_timeout)
//                             .unwrap();
//                     }
//                 }
//                 frames_rcvd => {
//                     log::debug!("receiver rx queue consumed {} frames", frames_rcvd);

//                     // Add frames back to fill queue
//                     while unsafe {
//                         xsk_rx
//                             .fq
//                             .produce_and_wakeup(
//                                 &rx_descs[..frames_rcvd],
//                                 &mut rx_fd,
//                                 config.poll_ms_timeout,
//                             )
//                             .unwrap()
//                     } != frames_rcvd
//                     {
//                         // Loop until frames added to the fill ring.
//                         log::debug!("receiver fill queue failed to allocate");
//                     }

//                     log::debug!("submitted {} frames to receiver fill queue", frames_rcvd);

//                     total_frames_rcvd += frames_rcvd;

//                     log::debug!("total frames received: {}", total_frames_rcvd);
//                 }
//             }
//         }

//         if total_frames_sent < config.num_frames_to_send
//             || total_frames_consumed < config.num_frames_to_send
//         {
//             // Handle tx
//             match unsafe { xsk_tx.cq.consume(&mut tx_descs[..]) } {
//                 0 => {
//                     log::debug!("sender comp queue consumed 0 frames");

//                     if xsk_tx.tx_q.needs_wakeup() {
//                         log::debug!("waking up sender tx queue");
//                         xsk_rx.tx_q.wakeup().unwrap();
//                     }
//                 }
//                 frames_rcvd => {
//                     log::debug!("sender comp queue consumed {} frames", frames_rcvd);

//                     total_frames_consumed += frames_rcvd;

//                     if total_frames_sent < config.num_frames_to_send {
//                         // Write new data
//                         tx_descs[..frames_rcvd].iter_mut().for_each(|desc| {
//                             let pkt = pkts.next().unwrap();

//                             unsafe {
//                                 xsk_tx.umem.frame_data_mut(desc).write_all(&pkt).unwrap();
//                             }
//                         });

//                         // Wait until we're ok to write
//                         while !tx_fd
//                             .poll(PollEvent::Write, config.poll_ms_timeout)
//                             .unwrap()
//                         {
//                             log::debug!("sender socket not ready to write");
//                             continue;
//                         }

//                         let frames_to_send = cmp::min(
//                             frames_rcvd,
//                             cmp::min(
//                                 config.max_batch_size,
//                                 config.num_frames_to_send - total_frames_sent,
//                             ),
//                         );

//                         // Add consumed frames back to the tx queue
//                         while unsafe {
//                             xsk_tx
//                                 .tx_q
//                                 .produce_and_wakeup(&tx_descs[..frames_to_send])
//                                 .unwrap()
//                         } != frames_to_send
//                         {
//                             // Loop until frames added to the tx ring.
//                             log::debug!("sender tx queue failed to allocate");
//                         }
//                         log::debug!("submitted {} frames to sender tx queue", frames_to_send);

//                         total_frames_sent += frames_to_send;
//                     }

//                     log::debug!("total frames consumed: {}", total_frames_consumed);
//                     log::debug!("total frames sent: {}", total_frames_sent);
//                 }
//             }
//         }
//     }

//     let elapsed_secs = start.elapsed().as_secs_f64();

//     // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
//     let pkt_len = pkts.next().unwrap().len();

//     let bytes_sent_per_sec: f64 = (total_frames_sent as f64) * (pkt_len as f64) / elapsed_secs;
//     let bytes_rcvd_per_sec: f64 = (total_frames_rcvd as f64) * (pkt_len as f64) / elapsed_secs;

//     // 1 bit/second = 1e-9 Gbps
//     // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
//     let gbps_sent = bytes_sent_per_sec / 0.125e9;
//     let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

//     // Note that this is being
//     println!(
//         "time taken to send {} {}-byte eth frames: {:.3} secs",
//         config.num_frames_to_send, pkt_len, elapsed_secs
//     );
//     println!(
//         "send throughput: {:.3} Gbps (eth frames sent: {})",
//         gbps_sent, total_frames_sent
//     );
//     println!(
//         "recv throughout: {:.3} Gbps (eth frames rcvd: {})",
//         gbps_rcvd, total_frames_rcvd
//     );
//     println!(
//         "note that these numbers are not reflective of actual AF_XDP socket performance,
// since packets are being sent over a VETH pair, and so pass through the kernel"
//     );
// }

// fn f(config: &Config, mut tx: (Xsk, PacketGenerator), mut rx: (Xsk, PacketGenerator)) {
//     let (xsk_tx, pkt_gen) = tx;
//     let (xsk_rx, _) = rx;

//     let tx_cfg = config.sender;
//     let rx_cfg = config.receiver;

//     let num_frames_to_send = config.num_frames_to_send;
//     let poll_ms_timeout = config.poll_ms_timeout;

//     let tx_descs = &mut xsk_tx.descs;
//     let rx_descs = &mut xsk_rx.descs;

//     let tx_fd = xsk_tx.tx_q.fd();
//     let rx_fd = xsk_rx.rx_q.fd();

//     let (begin_send_tx, begin_send_rx): (Sender<()>, Receiver<()>) = crossbeam_channel::bounded(1);

//     let start = Instant::now();

//     // Packets to write
//     let pkts = iter::repeat_with(|| {
//         pkt_gen
//             .generate_packet(1234, 1234, config.payload_size)
//             .unwrap()
//     });

//     let rx_handle = thread::spawn(move || {
//         // Populate receiver fill queue
//         let frames_filled = unsafe {
//             xsk_rx
//                 .fq
//                 .produce(&rx_descs[..rx_cfg.fq_size.get() as usize])
//         };

//         assert_eq!(frames_filled, rx_cfg.fq_size.get() as usize);

//         log::debug!("frames added to receiver fill queue: {}", frames_filled);

//         if let Err(_) = begin_send_tx.send(()) {
//             println!("sender thread has gone away");
//             return 0;
//         }

//         let mut total_frames_rcvd = 0;

//         while total_frames_rcvd < num_frames_to_send {
//             // In copy mode tx is driven by a syscall, so we need to
//             // wakeup the kernel with a call to either sendto() or
//             // poll() (wakeup() below uses sendto()).
//             if xsk_tx.tx_q.needs_wakeup() {
//                 log::debug!("waking up sender tx queue");
//                 xsk_tx.tx_q.wakeup().unwrap();
//             }

//             // Handle rx
//             match unsafe {
//                 xsk_rx
//                     .rx_q
//                     .poll_and_consume(&mut tx_descs[..], poll_ms_timeout)
//                     .unwrap()
//             } {
//                 0 => {
//                     // No frames consumed, wake up fill queue if required
//                     log::debug!("receiver rx queue consumed 0 frames");

//                     if xsk_rx.fq.needs_wakeup() {
//                         log::debug!("waking up receiver fill queue");
//                         xsk_rx.fq.wakeup(&mut rx_fd, poll_ms_timeout).unwrap();
//                     }

//                     // Or it might be that there are no packets left to receive
//                     if SENDER_DONE.load(Ordering::Relaxed) {
//                         break;
//                     }
//                 }
//                 frames_rcvd => {
//                     log::debug!("receiver rx queue consumed {} frames", frames_rcvd);

//                     // Add frames back to fill queue
//                     while unsafe {
//                         xsk_rx
//                             .fq
//                             .produce_and_wakeup(
//                                 &rx_descs[..frames_rcvd],
//                                 &mut rx_fd,
//                                 poll_ms_timeout,
//                             )
//                             .unwrap()
//                     } != frames_rcvd
//                     {
//                         // Loop until frames added to the fill ring.
//                         log::debug!("receiver fill queue failed to allocate");
//                     }

//                     log::debug!("submitted {} frames to receiver fill queue", frames_rcvd);

//                     total_frames_rcvd += frames_rcvd;

//                     log::debug!("total frames received: {}", total_frames_rcvd);
//                 }
//             }
//         }

//         log::debug!("receiver complete");
//         total_frames_rcvd
//     });

//     let tx_handle = thread::spawn(move || {});

//     // Populate receiver fill queue
//     let frames_filled = unsafe {
//         xsk_rx
//             .fq
//             .produce(&rx_descs[..rx_cfg.fq_size.get() as usize])
//     };

//     assert_eq!(frames_filled, rx_cfg.fq_size.get() as usize);

//     log::debug!("frames added to receiver fill queue: {}", frames_filled);

//     // Write packets to UMEM and populate sender tx queue
//     tx_descs[0..config.max_batch_size]
//         .iter_mut()
//         .for_each(|desc| {
//             let pkt = pkts.next().unwrap();

//             unsafe {
//                 xsk_tx.umem.frame_data_mut(desc).write_all(&pkt).unwrap();
//             }
//         });

//     let mut total_frames_sent = unsafe { xsk_tx.tx_q.produce(&tx_descs[..config.max_batch_size]) };

//     assert_eq!(total_frames_sent, config.max_batch_size);

//     log::debug!("frames added to sender tx queue: {}", total_frames_sent);

//     let mut total_frames_rcvd = 0;
//     let mut total_frames_consumed = 0;

//     while total_frames_consumed < config.num_frames_to_send
//         || total_frames_rcvd < config.num_frames_to_send
//     {
//         while total_frames_rcvd < total_frames_sent {
//             // In copy mode tx is driven by a syscall, so we need to
//             // wakeup the kernel with a call to either sendto() or
//             // poll() (wakeup() below uses sendto()).
//             if xsk_tx.tx_q.needs_wakeup() {
//                 log::debug!("waking up sender tx queue");
//                 xsk_tx.tx_q.wakeup().unwrap();
//             }

//             // Handle rx
//             match unsafe {
//                 xsk_rx
//                     .rx_q
//                     .poll_and_consume(&mut tx_descs[..], config.poll_ms_timeout)
//                     .unwrap()
//             } {
//                 0 => {
//                     // No frames consumed, wake up fill queue if required
//                     log::debug!("receiver rx queue consumed 0 frames");

//                     if xsk_rx.fq.needs_wakeup() {
//                         log::debug!("waking up receiver fill queue");
//                         xsk_rx
//                             .fq
//                             .wakeup(&mut rx_fd, config.poll_ms_timeout)
//                             .unwrap();
//                     }
//                 }
//                 frames_rcvd => {
//                     log::debug!("receiver rx queue consumed {} frames", frames_rcvd);

//                     // Add frames back to fill queue
//                     while unsafe {
//                         xsk_rx
//                             .fq
//                             .produce_and_wakeup(
//                                 &rx_descs[..frames_rcvd],
//                                 &mut rx_fd,
//                                 config.poll_ms_timeout,
//                             )
//                             .unwrap()
//                     } != frames_rcvd
//                     {
//                         // Loop until frames added to the fill ring.
//                         log::debug!("receiver fill queue failed to allocate");
//                     }

//                     log::debug!("submitted {} frames to receiver fill queue", frames_rcvd);

//                     total_frames_rcvd += frames_rcvd;

//                     log::debug!("total frames received: {}", total_frames_rcvd);
//                 }
//             }
//         }

//         if total_frames_sent < config.num_frames_to_send
//             || total_frames_consumed < config.num_frames_to_send
//         {
//             // Handle tx
//             match unsafe { xsk_tx.cq.consume(&mut tx_descs[..]) } {
//                 0 => {
//                     log::debug!("sender comp queue consumed 0 frames");

//                     if xsk_tx.tx_q.needs_wakeup() {
//                         log::debug!("waking up sender tx queue");
//                         xsk_rx.tx_q.wakeup().unwrap();
//                     }
//                 }
//                 frames_rcvd => {
//                     log::debug!("sender comp queue consumed {} frames", frames_rcvd);

//                     total_frames_consumed += frames_rcvd;

//                     if total_frames_sent < config.num_frames_to_send {
//                         // Write new data
//                         tx_descs[..frames_rcvd].iter_mut().for_each(|desc| {
//                             let pkt = pkts.next().unwrap();

//                             unsafe {
//                                 xsk_tx.umem.frame_data_mut(desc).write_all(&pkt).unwrap();
//                             }
//                         });

//                         // Wait until we're ok to write
//                         while !tx_fd
//                             .poll(PollEvent::Write, config.poll_ms_timeout)
//                             .unwrap()
//                         {
//                             log::debug!("sender socket not ready to write");
//                             continue;
//                         }

//                         let frames_to_send = cmp::min(
//                             frames_rcvd,
//                             cmp::min(
//                                 config.max_batch_size,
//                                 config.num_frames_to_send - total_frames_sent,
//                             ),
//                         );

//                         // Add consumed frames back to the tx queue
//                         while unsafe {
//                             xsk_tx
//                                 .tx_q
//                                 .produce_and_wakeup(&tx_descs[..frames_to_send])
//                                 .unwrap()
//                         } != frames_to_send
//                         {
//                             // Loop until frames added to the tx ring.
//                             log::debug!("sender tx queue failed to allocate");
//                         }
//                         log::debug!("submitted {} frames to sender tx queue", frames_to_send);

//                         total_frames_sent += frames_to_send;
//                     }

//                     log::debug!("total frames consumed: {}", total_frames_consumed);
//                     log::debug!("total frames sent: {}", total_frames_sent);
//                 }
//             }
//         }
//     }

//     let elapsed_secs = start.elapsed().as_secs_f64();

//     // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
//     let pkt_len = pkts.next().unwrap().len();

//     let bytes_sent_per_sec: f64 = (total_frames_sent as f64) * (pkt_len as f64) / elapsed_secs;
//     let bytes_rcvd_per_sec: f64 = (total_frames_rcvd as f64) * (pkt_len as f64) / elapsed_secs;

//     // 1 bit/second = 1e-9 Gbps
//     // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
//     let gbps_sent = bytes_sent_per_sec / 0.125e9;
//     let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

//     // Note that this is being
//     println!(
//         "time taken to send {} {}-byte eth frames: {:.3} secs",
//         config.num_frames_to_send, pkt_len, elapsed_secs
//     );
//     println!(
//         "send throughput: {:.3} Gbps (eth frames sent: {})",
//         gbps_sent, total_frames_sent
//     );
//     println!(
//         "recv throughout: {:.3} Gbps (eth frames rcvd: {})",
//         gbps_rcvd, total_frames_rcvd
//     );
//     println!(
//         "note that these numbers are not reflective of actual AF_XDP socket performance,
// since packets are being sent over a VETH pair, and so pass through the kernel"
//     );
// }

// /// Send ETH frames with payload size `msg_size` through dev2 to be received by dev1.
// /// Handle frame transmission and receipt in separate threads.
// fn dev1_to_dev2_multithreaded(
//     config: &Config,
//     mut dev1: (Xsk, PacketGenerator),
//     mut dev2: (Xsk, PacketGenerator),
// ) {
//     // Extra 42 bytes is for the eth and IP headers
//     let sent_eth_frame_size = config.payload_size + 42;

//     // Make copies for the separate threads
//     let config_rx = config.clone();
//     let config_tx = config.clone();

//     let veth_config = veth_config.clone();

//     let (d1_to_d2_tx, d1_to_d2_rx): (Sender<()>, Receiver<()>) = crossbeam_channel::bounded(1);

//     let start = Instant::now();

//     // Spawn the receiver thread
//     let rx_handle = thread::spawn(move || {
//         let dev1_frames = &mut dev1.frames;

//         // Populate fill queue
//         let frames_filled = unsafe {
//             dev1.fill_q
//                 .produce_and_wakeup(
//                     &dev1_frames[..config_rx.fill_q_size as usize],
//                     dev1.rx_q.fd(),
//                     config_rx.poll_ms_timeout,
//                 )
//                 .unwrap()
//         };

//         assert_eq!(frames_filled, config_rx.fill_q_size as usize);
//         log::debug!("(dev1) init frames added to dev1.fill_q: {}", frames_filled);

//         if let Err(_) = d1_to_d2_tx.send(()) {
//             println!("sender thread (dev2) has gone away");
//             return 0;
//         }

//         let mut total_frames_rcvd = 0;

//         while total_frames_rcvd < config_rx.num_frames_to_send {
//             match dev1
//                 .rx_q
//                 .poll_and_consume(&mut dev1_frames[..], config_rx.poll_ms_timeout)
//                 .unwrap()
//             {
//                 0 => {
//                     log::debug!("(dev1) dev1.rx_q.poll_and_consume() consumed 0 frames");
//                     // No packets consumed, wake up fill queue if required
//                     if dev1.fill_q.needs_wakeup() {
//                         log::debug!("(dev1) waking up dev1.fill_q");
//                         dev1.fill_q
//                             .wakeup(dev1.rx_q.fd(), config_rx.poll_ms_timeout)
//                             .unwrap();
//                     }

//                     // Or it might be that there are no packets left to receive
//                     if SENDER_DONE.load(Ordering::Relaxed) {
//                         break;
//                     }
//                 }
//                 frames_rcvd => {
//                     log::debug!(
//                         "(dev1) dev1.rx_q.poll_and_consume() consumed {} frames",
//                         frames_rcvd
//                     );
//                     // Add frames back to fill queue
//                     while unsafe {
//                         dev1.fill_q
//                             .produce_and_wakeup(
//                                 &dev1_frames[..frames_rcvd],
//                                 dev1.rx_q.fd(),
//                                 config_rx.poll_ms_timeout,
//                             )
//                             .unwrap()
//                     } != frames_rcvd
//                     {
//                         // Loop until frames added to the fill ring.
//                         log::debug!("(dev1) dev1.fill_q.produce_and_wakeup() failed to allocate");
//                     }

//                     log::debug!(
//                         "(dev1) dev1.fill_q.produce_and_wakeup() submitted {} frames",
//                         frames_rcvd
//                     );

//                     total_frames_rcvd += frames_rcvd;
//                     log::debug!("(dev1) total frames received: {}", total_frames_rcvd);
//                 }
//             }
//         }

//         log::debug!("(dev1) recv complete");
//         total_frames_rcvd
//     });

//     // Spawn the sender thread
//     let tx_handle = thread::spawn(move || {
//         let dev2_frames = &mut dev2.frames;

//         // Populate tx queue.
//         let mut total_frames_consumed = 0;
//         let mut total_frames_sent = unsafe {
//             dev2.tx_q
//                 .produce_and_wakeup(&dev2_frames[..config_tx.max_batch_size])
//                 .unwrap()
//         };

//         assert_eq!(total_frames_sent, config_tx.max_batch_size);
//         log::debug!(
//             "(dev2) init frames added to dev2.tx_q: {}",
//             total_frames_sent
//         );

//         // Let the receiver populate its fill queue first and wait for the go-ahead.
//         if let Err(_) = d1_to_d2_rx.recv() {
//             println!("receiver thread (dev1) has gone away");
//             return 0;
//         }

//         while total_frames_consumed < config_tx.num_frames_to_send {
//             match dev2.comp_q.consume(&mut dev2_frames[..]) {
//                 0 => {
//                     log::debug!("(dev2) dev2.comp_q.consume() consumed 0 frames");
//                     // In copy mode so need to make a syscall to actually send packets
//                     // despite having produced frames onto the fill ring.
//                     if dev2.tx_q.needs_wakeup() {
//                         log::debug!("(dev2) waking up dev2.tx_q");
//                         dev2.tx_q.wakeup().unwrap();
//                     }
//                 }
//                 frames_rcvd => {
//                     log::debug!(
//                         "(dev2) dev2.comp_q.consume() consumed {} frames",
//                         frames_rcvd
//                     );

//                     total_frames_consumed += frames_rcvd;

//                     if total_frames_sent < config_tx.num_frames_to_send {
//                         dev2_frames[..frames_rcvd].iter_mut().for_each(|frame| {
//                             let eth_frame =
//                                 setup::generate_eth_frame(&veth_config, config_tx.payload_size);
//                             unsafe {
//                                 frame.write_all(&eth_frame).unwrap();
//                             }
//                             assert_eq!(frame.desc().len(), eth_frame.len());
//                         });

//                         // Wait until we're ok to write
//                         while !dev2
//                             .tx_q
//                             .fd()
//                             .poll(PollEvent::Write, config_tx.poll_ms_timeout)
//                             .unwrap()
//                         {
//                             log::debug!("(dev2) poll_write(dev2.tx_q) returned false");
//                             continue;
//                         }

//                         let frames_to_send = cmp::min(
//                             cmp::min(frames_rcvd, config_tx.max_batch_size),
//                             config_tx.num_frames_to_send - total_frames_sent,
//                         );

//                         // Add consumed frames back to the tx queue
//                         while unsafe {
//                             dev2.tx_q
//                                 .produce_and_wakeup(&dev2_frames[..frames_to_send])
//                                 .unwrap()
//                         } != frames_to_send
//                         {
//                             // Loop until frames added to the tx ring.
//                             log::debug!("(dev2) dev2.tx_q.produce_and_wakeup() failed to allocate");
//                         }

//                         log::debug!(
//                             "(dev2) dev2.tx_q.produce_and_wakeup() submitted {} frames",
//                             frames_to_send
//                         );

//                         total_frames_sent += frames_to_send;
//                     }

//                     log::debug!("(dev2) total frames consumed: {}", total_frames_consumed);
//                     log::debug!("(dev2) total frames sent: {}", total_frames_sent);
//                 }
//             }
//         }

//         log::debug!("(dev2) send complete");

//         // Mark sender as done so receiver knows when to return
//         SENDER_DONE.store(true, Ordering::Relaxed);

//         total_frames_consumed
//     });

//     let tx_res = tx_handle.join();
//     let rx_res = rx_handle.join();

//     if let (Ok(pkts_sent), Ok(pkts_rcvd)) = (&tx_res, &rx_res) {
//         let elapsed_secs = start.elapsed().as_secs_f64();

//         // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
//         let bytes_sent_per_sec: f64 =
//             (*pkts_sent as f64) * (sent_eth_frame_size as f64) / elapsed_secs;
//         let bytes_rcvd_per_sec: f64 =
//             (*pkts_rcvd as f64) * (sent_eth_frame_size as f64) / elapsed_secs;

//         // 1 bit/second = 1e-9 Gbps
//         // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
//         let gbps_sent = bytes_sent_per_sec / 0.125e9;
//         let gbps_rcvd = bytes_rcvd_per_sec / 0.125e9;

//         println!(
//             "time taken to send {} {}-byte eth frames: {:.3} secs",
//             config.num_frames_to_send, sent_eth_frame_size, elapsed_secs
//         );
//         println!(
//             "send throughput: {:.3} Gbps (eth frames sent: {})",
//             gbps_sent, pkts_sent
//         );
//         println!(
//             "recv throughout: {:.3} Gbps (eth frames rcvd: {})",
//             gbps_rcvd, pkts_rcvd
//         );
//         println!(
//             "note that these numbers are not reflective of actual AF_XDP socket performance,
// since packets are being sent over a VETH pair, and so pass through the kernel"
//         );
//     } else {
//         println!("error (tx_res: {:?}) (rx_res: {:?})", tx_res, rx_res);
//     }
// }

// pub fn build_socket_and_umem(
//     umem_config: UmemConfig,
//     socket_config: SocketConfig,
//     frame_count: u32,
//     if_name: &Interface,
//     queue_id: u32,
// ) -> Xsk {
//     let (umem, descs) = Umem::new(umem_config, frame_count, false).expect("failed to build umem");

//     let (tx_q, rx_q, fq_and_cq) =
//         Socket::new(socket_config, &umem, if_name, queue_id).expect("failed to build socket");

//     let (fq, cq) = fq_and_cq.expect(&format!(
//         "missing fill and comp queue - interface {:?} may already be bound to",
//         if_name
//     ));

//     Xsk {
//         umem,
//         fq,
//         cq,
//         tx_q,
//         rx_q,
//         descs,
//     }
// }

// fn build_umem_and_socket_config(config: &XskConfig) -> (UmemConfig, SocketConfig) {
//     let umem_config = UmemConfig::builder()
//         .frame_size(config.frame_size)
//         .fill_queue_size(config.fq_size)
//         .comp_queue_size(config.cq_size)
//         .frame_size(config.frame_size)
//         .build()
//         .unwrap();

//     let socket_config = SocketConfig::builder()
//         .rx_queue_size(config.rx_q_size)
//         .tx_queue_size(config.tx_q_size)
//         .bind_flags(BindFlags::XDP_USE_NEED_WAKEUP)
//         .build();

//     (umem_config, socket_config)
// }

// fn run_example(
//     config: Config,
//     dev_tx: (VethDevConfig, PacketGenerator),
//     dev_rx: (VethDevConfig, PacketGenerator),
// ) {
//     let (umem_config_tx, socket_config_tx) = build_umem_and_socket_config(&config.sender);
//     let (umem_config_rx, socket_config_rx) = build_umem_and_socket_config(&config.receiver);

//     let xsk_tx = build_socket_and_umem(
//         umem_config_tx.clone(),
//         socket_config_tx.clone(),
//         config.sender.frame_count,
//         &dev_tx.0.if_name().parse().unwrap(),
//         0,
//     );

//     let xsk_rx = build_socket_and_umem(
//         umem_config_rx.clone(),
//         socket_config_rx.clone(),
//         config.sender.frame_count,
//         &dev_rx.0.if_name().parse().unwrap(),
//         0,
//     );

//     if config.is_multithreaded {
//         println!(
//             "sending {} eth frames w/ {}-byte payload (total msg size: {} bytes) (multi-threaded)",
//             config.num_frames_to_send,
//             config.payload_size,
//             &config.sender.frame_size.get()
//         );
//         dev1_to_dev2_multithreaded(&config, (xsk_tx, dev_tx.1), (xsk_rx, dev_rx.1));
//     } else {
//         println!(
//             "sending {} eth frames w/ {}-byte payload (total msg size: {} bytes) (single-threaded)",
//             config.num_frames_to_send,
//             config.payload_size,
//             &config.sender.frame_size.get()
//         );
//         dev1_to_dev2_single_thread(&config, (xsk_tx, dev_tx.1), (xsk_rx, dev_rx.1));
//     }
// }

// fn main() {
//     env_logger::init();

//     let config = Config::from_opt(Opt::from_args());

//     let dev1_config = VethDevConfig {
//         if_name: "xsk_test_dev1".into(),
//         addr: [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a],
//         ip_addr: LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
//     };

//     let dev2_config = VethDevConfig {
//         if_name: "xsk_test_dev2".into(),
//         addr: [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31],
//         ip_addr: LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
//     };

//     // We'll keep track of ctrl+c events but not let them kill the process
//     // immediately as we may need to clean up the veth pair.
//     let ctrl_c_events = util::ctrl_channel().unwrap();

//     let (complete_tx, complete_rx) = crossbeam_channel::bounded(1);

//     let runtime = Runtime::new().unwrap();

//     let example_handle = thread::spawn(move || {
//         let res = runtime.block_on(veth_setup::run_with_veth_pair(
//             dev1_config,
//             dev2_config,
//             move |dev1, dev2| run_example(config, dev2, dev2),
//         ));

//         let _ = complete_tx.send(());

//         res
//     });

//     // Wait for either the example to finish or for a ctrl+c event to occur
//     crossbeam_channel::select! {
//         recv(complete_rx) -> _ => {
//         },
//         recv(ctrl_c_events) -> _ => {
//             println!("SIGINT received");
//         }
//     }

//     example_handle.join().unwrap().unwrap();
// }

fn main() {
    println!("hello")
}
