use bench::{util, Config, Role, XskState};
use clap::{App, Arg};
use xsk_rs::FrameDesc;

fn recv(config: &Config, mut xsk: XskState, mut frame_descs: Vec<FrameDesc>) -> usize {
    let num_frames_to_process: usize = *config.num_frames_to_process() as usize;

    // Keep track of socket file descriptor as we need to pass it to poll
    let mut fd = xsk.rx_q().fd().clone();

    // Populate fill queue
    assert_eq!(
        xsk.fill_q()
            .produce(&frame_descs[..*config.fill_q_size() as usize]),
        *config.fill_q_size() as usize
    );

    let mut total_frames_rcvd = 0;

    while total_frames_rcvd < num_frames_to_process {
        match xsk
            .rx_q()
            .poll_and_consume(&mut frame_descs[..], *config.poll_ms_timeout())
            .unwrap()
        {
            0 => {
                log::debug!("xsk.rx_q.poll_and_consume() consumed 0 frames");
                // No packets consumed, wake up fill queue if required
                if xsk.fill_q().needs_wakeup() {
                    log::debug!("waking up xsk.fill_q");
                    xsk.fill_q()
                        .wakeup(&mut fd, *config.poll_ms_timeout())
                        .unwrap();
                }
            }
            frames_rcvd => {
                log::debug!(
                    "xsk.rx_q.poll_and_consume() consumed {} frames",
                    frames_rcvd
                );
                // Add frames back to fill queue
                while xsk
                    .fill_q()
                    .produce_and_wakeup(
                        &frame_descs[..frames_rcvd],
                        &mut fd,
                        *config.poll_ms_timeout(),
                    )
                    .unwrap()
                    != frames_rcvd
                {
                    // Loop until frames added to the fill ring.
                    log::debug!("xsk.fill_q.produce_and_wakeup() failed to allocate");
                }

                total_frames_rcvd += frames_rcvd;
                log::debug!("total frames received: {}", total_frames_rcvd);
            }
        }
    }

    total_frames_rcvd
}

fn get_args() -> Config {
    let matches = App::new("xsk_bench_rx")
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
        .get_matches();

    let if_name = matches.value_of("if_name").unwrap();
    let if_queue = util::parse_arg_with_default(&matches, "if_queue", 0).unwrap();
    let use_need_wakeup = matches.is_present("use_need_wakeup");
    let zerocopy = matches.is_present("zerocopy");
    let drv_mode = matches.is_present("drv_mode");
    let num_frames_to_process =
        util::parse_arg_with_default(&matches, "num_frames_to_process", 10_000_000).unwrap();

    Config::new(
        if_name.into(),
        if_queue,
        use_need_wakeup,
        zerocopy,
        drv_mode,
        num_frames_to_process,
    )
}

fn main() {
    env_logger::init();

    let config = get_args();

    let (umem_config, xsk_config) = util::build_xsk_configs(&config).unwrap();

    let (xsk_state, frame_descs) = util::build_socket_and_umem(
        umem_config,
        xsk_config,
        config.if_name(),
        *config.if_queue(),
    )
    .unwrap();

    util::handle_sync(Role::Rx).expect("sync with TX process failed");

    println!("synced with TX, running RX bench with the following settings:");
    println!("{:#?}", config);

    let f = move |c: &Config| recv(&c, xsk_state, frame_descs);

    util::run_bench(&config, Role::Rx, f);
}
