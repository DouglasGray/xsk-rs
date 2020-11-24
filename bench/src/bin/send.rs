use bench::{net, util, Config, NetConfig, Role, XskState};
use clap::{App, Arg};
use std::{cmp, convert::TryInto};
use xsk_rs::{socket, FrameDesc, Umem};

fn send(config: &Config, mut xsk: XskState, mut frame_descs: Vec<FrameDesc>) -> usize {
    let num_frames_to_process: usize = (*config.num_frames_to_process()).try_into().unwrap();
    let max_batch_size: usize = (*config.max_batch_size()).try_into().unwrap();
    let eth_frame_len = 42 + config.pkt_payload_len();

    // Keep track of socket file descriptor as we need to pass it to poll
    let mut fd = xsk.rx_q().fd().clone();

    // Populate tx queue.
    let mut total_frames_consumed = 0;
    let mut total_frames_sent = xsk
        .tx_q()
        .produce_and_wakeup(&frame_descs[..max_batch_size])
        .unwrap();

    assert_eq!(total_frames_sent, max_batch_size);

    while total_frames_consumed < num_frames_to_process {
        match xsk.comp_q().consume(&mut frame_descs[..]) {
            0 => {
                if xsk.tx_q().needs_wakeup() {
                    xsk.tx_q().wakeup().unwrap();
                }
            }
            frames_rcvd => {
                total_frames_consumed += frames_rcvd;

                if total_frames_sent < num_frames_to_process {
                    // Data is still contained in the frames so just set the descriptor's length
                    for desc in frame_descs[..frames_rcvd].iter_mut() {
                        desc.set_len(eth_frame_len);
                    }

                    // Wait until we're ok to write
                    while !socket::poll_write(&mut fd, *config.poll_ms_timeout()).unwrap() {
                        continue;
                    }

                    let frames_to_send = cmp::min(
                        cmp::min(frames_rcvd, max_batch_size),
                        num_frames_to_process - total_frames_sent,
                    );

                    // Add consumed frames back to the tx queue
                    while xsk
                        .tx_q()
                        .produce_and_wakeup(&frame_descs[..frames_to_send])
                        .unwrap()
                        != frames_to_send
                    {
                        // Loop until frames added to the tx ring.
                    }

                    total_frames_sent += frames_to_send;
                }
            }
        }
    }

    total_frames_consumed
}

fn get_args() -> (Config, NetConfig) {
    let matches = App::new("xsk_bench_tx")
        .arg(
            Arg::with_name("if_name")
                .short("i")
                .long("if-name")
                .required(true)
                .takes_value(true)
                .help("name of interface to bind to"),
        )
        .arg(
            Arg::with_name("if_queue")
                .short("q")
                .long("if-queue")
                .required(false)
                .takes_value(true)
                .help("interface queue to bind to"),
        )
        .arg(
            Arg::with_name("use_need_wakeup")
                .short("w")
                .long("need-wakeup")
                .required(false)
                .takes_value(false)
                .help("Enable XDP_USE_NEED_WAKEUP (recommended if driver supports it)"),
        )
        .arg(
            Arg::with_name("zerocopy")
                .short("z")
                .long("zerocopy")
                .required(false)
                .takes_value(false)
                .help("Enable XDP_ZEROCOPY (recommended if driver supports it)"),
        )
        .arg(
            Arg::with_name("drv_mode")
                .short("d")
                .long("drv-mode")
                .required(false)
                .takes_value(false)
                .help("Enable XDP_FLAGS_DRV_MODE (recommended if driver supports XDP natively)"),
        )
        .arg(
            Arg::with_name("num_frames_to_process")
                .short("n")
                .long("num-frames-to-process")
                .required(false)
                .takes_value(true)
                .help("Set total number of frames to process"),
        )
        .arg(
            Arg::with_name("src_mac")
                .long("src-mac")
                .required(true)
                .takes_value(true)
                .help("Set source mac address"),
        )
        .arg(
            Arg::with_name("dst_mac")
                .long("dst-mac")
                .required(true)
                .takes_value(true)
                .help("Set destination mac address"),
        )
        .arg(
            Arg::with_name("src_ip")
                .long("src-ip")
                .required(true)
                .takes_value(true)
                .help("Set source ip address (IPv4)"),
        )
        .arg(
            Arg::with_name("dst_ip")
                .long("dst-ip")
                .required(true)
                .takes_value(true)
                .help("Set destination ip address (IPv4)"),
        )
        .arg(
            Arg::with_name("src_port")
                .long("src-port")
                .required(false)
                .takes_value(true)
                .help("Set source port"),
        )
        .arg(
            Arg::with_name("dst_port")
                .long("dst-port")
                .required(false)
                .takes_value(true)
                .help("Set destination port"),
        )
        .get_matches();

    let if_name = matches.value_of("if_name").unwrap();
    let if_queue = util::parse_arg_with_default(&matches, "if_queue", 0).unwrap();
    let use_need_wakeup = matches.is_present("use_need_wakup");
    let zerocopy = matches.is_present("zerocopy");
    let drv_mode = matches.is_present("drv_mode");
    let num_frames_to_process =
        util::parse_arg_with_default(&matches, "num_frames_to_process", 10_500_000).unwrap();

    let src_mac = util::parse_mac_addr(matches.value_of("src_mac").unwrap()).unwrap();
    let dst_mac = util::parse_mac_addr(matches.value_of("dst_mac").unwrap()).unwrap();
    let src_ip = util::parse_ip_addr(matches.value_of("src_ip").unwrap()).unwrap();
    let dst_ip = util::parse_ip_addr(matches.value_of("dst_ip").unwrap()).unwrap();
    let src_port = util::parse_arg_with_default(&matches, "src_port", 1234).unwrap();
    let dst_port = util::parse_arg_with_default(&matches, "dst_port", 1234).unwrap();

    let config = Config::new(
        if_name.into(),
        if_queue,
        use_need_wakeup,
        zerocopy,
        drv_mode,
        num_frames_to_process,
    );

    let net_config = NetConfig::new(src_mac, dst_mac, src_ip, dst_ip, src_port, dst_port);

    (config, net_config)
}

fn populate_umem(
    config: &Config,
    net_config: &NetConfig,
    umem: &mut Umem,
    frame_descs: &mut Vec<FrameDesc>,
) {
    let eth_frame = net::generate_eth_frame(net_config, *config.pkt_payload_len());

    for desc in frame_descs.iter_mut() {
        umem.copy_data_to_frame(desc, &eth_frame[..]).unwrap();
        assert_eq!(desc.len(), 42 + config.pkt_payload_len());
    }
}

fn main() {
    let (config, net_config) = get_args();

    let (umem_config, xsk_config) = util::build_xsk_configs(&config).unwrap();

    let (mut xsk_state, mut frame_descs) = util::build_socket_and_umem(
        umem_config,
        xsk_config,
        config.if_name(),
        *config.if_queue(),
    )
    .unwrap();

    populate_umem(
        &config,
        &net_config,
        &mut xsk_state.umem(),
        &mut frame_descs,
    );

    util::handle_sync(Role::Tx).expect("sync with RX process failed");

    println!("synced with RX, running TX bench with the following settings:");
    println!("{:#?}", config);
    println!("{:#?}", net_config);

    let f = move |c: &Config| send(&c, xsk_state, frame_descs);

    util::run_bench(&config, Role::Tx, f);
}
