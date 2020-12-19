use clap::ArgMatches;
use nix::{errno::Errno, sys::stat::Mode, unistd};
use std::{
    fmt::Debug,
    fs::OpenOptions,
    io::ErrorKind,
    num::NonZeroU32,
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use xsk_rs::{BindFlags, FrameDesc, LibbpfFlags, Socket, SocketConfig, Umem, UmemConfig, XdpFlags};

use super::{Config, Role, XskState};

pub fn handle_sync(role: Role) -> anyhow::Result<()> {
    let sync_file_path = "/tmp/xsk_rs_bench_12345.pipe";

    // TX is in charge of setting up its write end
    match role {
        Role::Tx => {
            // If TX then set up the named pipe and wait for
            // the RX process to open the other end.
            match unistd::mkfifo(sync_file_path, Mode::S_IWUSR | Mode::S_IROTH) {
                Ok(_) | Err(nix::Error::Sys(Errno::EEXIST)) => (),
                Err(err) => return Err(err.into()),
            }
            let _ = OpenOptions::new().write(true).open(&sync_file_path)?;
            Ok(())
        }
        Role::Rx => {
            // If RX loop until we can open the named pipe
            let max_attempts = 10;
            let mut attempts = 0;

            loop {
                match OpenOptions::new().read(true).open(&sync_file_path) {
                    Ok(_) => return Ok(()),
                    Err(err) => match err.kind() {
                        ErrorKind::NotFound => {
                            attempts += 1;

                            if attempts >= max_attempts {
                                return Err(anyhow::anyhow!(
                                    "Failed to sync with TX after {} attempts",
                                    max_attempts
                                ));
                            } else {
                                thread::sleep(Duration::from_secs(1));
                            }

                            continue;
                        }
                        _ => return Err(err.into()),
                    },
                }
            }
        }
    }
}

pub fn parse_arg_with_default<T>(matches: &ArgMatches, name: &str, default: T) -> anyhow::Result<T>
where
    T: FromStr,
    <T as FromStr>::Err: Debug,
{
    matches
        .value_of(name)
        .map(|s| {
            s.parse()
                .map_err(|e| anyhow::anyhow!("failed to parse {} arg: {:?}", name, e))
        })
        .unwrap_or(Ok(default))
}

pub fn build_socket_and_umem(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    if_name: &str,
    if_queue: u32,
) -> anyhow::Result<(XskState<'static>, Vec<FrameDesc>)> {
    let (mut umem, fill_q, comp_q, frame_descs) = Umem::builder(umem_config)
        .create_mmap()
        .map_err(|e| anyhow::anyhow!("failed to create mmap area for {}: {}", if_name, e))?
        .create_umem()
        .map_err(|e| anyhow::anyhow!("failed to create umem for {}: {}", if_name, e))?;

    let (tx_q, rx_q) = Socket::new(socket_config, &mut umem, &if_name, if_queue)
        .map_err(|e| anyhow::anyhow!("failed to build socket for {}: {}", if_name, e))?;

    Ok((
        XskState {
            fill_q,
            comp_q,
            tx_q,
            rx_q,
            umem,
        },
        frame_descs,
    ))
}

pub fn build_xsk_configs(config: &Config) -> anyhow::Result<(UmemConfig, SocketConfig)> {
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
    .map_err(|e| anyhow::anyhow!("failed to build umem config: {}", e))?;

    let mut xdp_flags = XdpFlags::empty();
    let mut bind_flags = BindFlags::empty();

    if config.use_need_wakeup {
        bind_flags |= BindFlags::XDP_USE_NEED_WAKEUP
    }
    if config.zerocopy {
        bind_flags |= BindFlags::XDP_ZEROCOPY
    }
    if config.drv_mode {
        xdp_flags |= XdpFlags::XDP_FLAGS_DRV_MODE
    }

    let socket_config = SocketConfig::new(
        config.rx_q_size,
        config.tx_q_size,
        LibbpfFlags::empty(),
        xdp_flags,
        bind_flags,
    )
    .map_err(|e| anyhow::anyhow!("failed to build socket config: {}", e))?;

    Ok((umem_config, socket_config))
}

pub fn parse_mac_addr(s: &str) -> anyhow::Result<[u8; 6]> {
    let parts: Vec<&str> = s.split(":").collect();

    if parts.len() != 6 {
        return Err(anyhow::anyhow!(
            "mac address {} is the wrong length, expected 6 parts got {}",
            s,
            parts.len()
        ));
    }

    let mut bytes = [0; 6];

    for (byte, hex) in bytes.iter_mut().zip(parts.iter()) {
        *byte = u8::from_str_radix(hex, 16)
            .map_err(|e| anyhow::anyhow!("failed to parse mac address {} at {}: {}", s, hex, e))?;
    }

    Ok(bytes)
}

pub fn parse_ip_addr(s: &str) -> anyhow::Result<[u8; 4]> {
    let parts: Vec<&str> = s.split(".").collect();

    if parts.len() != 4 {
        return Err(anyhow::anyhow!(
            "ip address {} is the wrong length, expected 4 parts got {}",
            s,
            parts.len()
        ));
    }

    let mut bytes = [0; 4];

    for (byte, dec) in bytes.iter_mut().zip(parts.iter()) {
        *byte = u8::from_str_radix(dec, 10)
            .map_err(|e| anyhow::anyhow!("failed to parse ip address {} at {}: {}", s, dec, e))?;
    }

    Ok(bytes)
}

pub fn run_bench<F>(config: &Config, role: Role, f: F)
where
    F: FnOnce(&Config) -> usize,
{
    let start = Instant::now();

    let frames_processed: f64 = f(&config) as f64;

    let elapsed_secs = start.elapsed().as_secs_f64();

    // Bytes sent per second is (number_of_packets * packet_size) / seconds_elapsed
    let sent_frame_size: f64 = 42f64 + *config.pkt_payload_len() as f64;

    let bytes_processed_per_sec = (frames_processed) * (sent_frame_size) / elapsed_secs;

    // 1 bit/second = 1e-9 Gbps
    // gbps_sent = (bytes_sent_per_sec * 8) / 1e9 = bytes_sent_per_sec / 0.125e9
    let gbps = bytes_processed_per_sec / 0.125e9;

    println!(
        "time taken for {:?} to process {} {}-byte eth frames: {:.3} secs",
        role,
        config.num_frames_to_process(),
        sent_frame_size,
        elapsed_secs
    );
    println!(
        "{:?} throughput: {:.3} Gbps (eth frames processed: {})",
        role, gbps, frames_processed
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_mac_addr() {
        let bytes = [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a];

        let addr = "f6:e0:f6:c9:60:0a";

        assert_eq!(parse_mac_addr(&addr).unwrap(), bytes);
    }

    #[test]
    fn should_parse_ip_addr() {
        let bytes = [0xc0, 0xa8, 0x45, 0x01];

        let addr = "192.168.69.1";

        assert_eq!(parse_ip_addr(&addr).unwrap(), bytes);
    }
}
