use std::{convert::TryInto, io::Write, net::Ipv4Addr, num::NonZeroU32, str, thread};
use tokio::runtime::Runtime;
use xsk_rs::{
    config::{Interface, SocketConfig, UmemConfig},
    socket::{RxQueue, Socket, TxQueue},
    umem::{frame::Frame, CompQueue, FillQueue, Umem},
};

#[allow(dead_code)]
mod setup;
use setup::{util, veth_setup, LinkIpAddr, PacketGenerator, VethDevConfig};

struct Xsk {
    _umem: Umem,
    fq: FillQueue,
    _cq: CompQueue,
    tx_q: TxQueue,
    rx_q: RxQueue,
    frames: Vec<Frame>,
}

fn build_socket_and_umem<'a, 'umem>(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    frame_count: NonZeroU32,
    if_name: &Interface,
    queue_id: u32,
) -> Xsk {
    let (umem, descs) = Umem::new(umem_config, frame_count, false).expect("failed to create UMEM");

    let (tx_q, rx_q, fq_and_cq) =
        Socket::new(socket_config, &umem, if_name, queue_id).expect("failed to create socket");

    let (fq, cq) = fq_and_cq.expect("fill queue and comp queue were None, expected Some");

    Xsk {
        _umem: umem,
        fq,
        _cq: cq,
        tx_q,
        rx_q,
        frames: descs,
    }
}

fn hello_xdp(dev1: (VethDevConfig, PacketGenerator), dev2: (VethDevConfig, PacketGenerator)) {
    let dev1_if = dev1.0.if_name().parse().unwrap();
    let dev2_if = dev2.0.if_name().parse().unwrap();

    let mut dev1 = build_socket_and_umem(
        UmemConfig::default(),
        SocketConfig::default(),
        16.try_into().unwrap(),
        &dev1_if,
        0,
    );

    let mut dev2 = build_socket_and_umem(
        UmemConfig::default(),
        SocketConfig::default(),
        16.try_into().unwrap(),
        &dev2_if,
        0,
    );

    // Want to send some data from dev1 to dev2. So we need to:
    // 1. Make sure that dev2 can receive data by adding frames to its FillQueue
    // 2. Update the dev1's UMEM with the data we want to send and update
    //    the corresponding frame descriptor with the data's length
    // 3. Hand over the frame to the kernel for transmission
    // 4. Read from dev2

    // 1. Add frames to dev2's fill queue
    assert_eq!(unsafe { dev2.fq.produce(&dev2.frames) }, dev2.frames.len());

    // 2. Update dev1's UMEM with the data we want to send and update the frame desc
    let pkt = "Hello, world!".as_bytes();

    unsafe {
        dev1.frames[0]
            .data_mut()
            .cursor()
            .write_all(pkt)
            .expect("failed writing packet to frame")
    }

    assert_eq!(dev1.frames[0].len(), pkt.len());

    // 3. Hand over the frame to the kernel for transmission
    println!("sending: {:?}", str::from_utf8(&pkt).unwrap());

    assert_eq!(
        unsafe { dev1.tx_q.produce_and_wakeup(&dev1.frames[..1]).unwrap() },
        1
    );

    // 4. Read from dev2
    let pkts_recvd = unsafe { dev2.rx_q.poll_and_consume(&mut dev2.frames, 100).unwrap() };

    // Check that one of the packets we received matches what we expect.
    for recv_frame in dev2.frames.iter().take(pkts_recvd) {
        let frame_data = unsafe { recv_frame.data() };

        println!(
            "received: {:?}",
            str::from_utf8(frame_data.contents()).unwrap()
        );

        if frame_data.contents() == &pkt[..] {
            return;
        }
    }

    panic!("no matching frames received")
}

fn main() {
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
            hello_xdp,
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
