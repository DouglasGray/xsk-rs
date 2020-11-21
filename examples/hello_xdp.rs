mod setup;

use std::{net::Ipv4Addr, num::NonZeroU32, str, thread};
use tokio::{
    runtime::Runtime,
    sync::oneshot::{self, error::TryRecvError},
};
use xsk_rs::{FillQueue, FrameDesc, RxQueue, Socket, SocketConfig, TxQueue, Umem, UmemConfig};

use setup::{LinkIpAddr, VethConfig};

// Put umem at bottom so drop order is correct
struct SocketState<'umem> {
    fill_q: FillQueue<'umem>,
    tx_q: TxQueue<'umem>,
    rx_q: RxQueue<'umem>,
    frame_descs: Vec<FrameDesc>,
    umem: Umem<'umem>,
}

fn build_socket_and_umem<'a, 'umem>(
    umem_config: UmemConfig,
    socket_config: SocketConfig,
    if_name: &'a str,
    queue_id: u32,
) -> SocketState {
    let (mut umem, fill_q, _comp_q, frame_descs) = Umem::builder(umem_config)
        .create_mmap()
        .expect(format!("failed to create mmap area for {}", if_name).as_str())
        .create_umem()
        .expect(format!("failed to create umem for {}", if_name).as_str());

    let (tx_q, rx_q) = Socket::new(socket_config, &mut umem, if_name, queue_id)
        .expect(format!("failed to build socket for {}", if_name).as_str());

    SocketState {
        umem,
        fill_q,
        tx_q,
        rx_q,
        frame_descs,
    }
}

fn hello_xdp(veth_config: &VethConfig) {
    // Create umem and socket configs
    let umem_config = UmemConfig::default(NonZeroU32::new(16).unwrap(), false);
    let socket_config = SocketConfig::default();

    let mut dev1 = build_socket_and_umem(
        umem_config.clone(),
        socket_config.clone(),
        &veth_config.dev1_name(),
        0,
    );

    let mut dev2 = build_socket_and_umem(umem_config, socket_config, &veth_config.dev2_name(), 0);

    let mut dev1_frames = dev1.frame_descs;

    let mut dev2_frames = dev2.frame_descs;

    // Want to send some data from dev1 to dev2. So we need to:
    // 1. Make sure that dev2 can receive data by adding frames to its FillQueue
    // 2. Update the dev1's UMEM with the data we want to send and update
    //    the corresponding frame descriptor with the data's length
    // 3. Hand over the frame to the kernel for transmission
    // 4. Read from dev2

    // 1. Add frames to dev2's FillQueue
    assert_eq!(dev2.fill_q.produce(&dev2_frames[..]), dev2_frames.len());

    // 2. Update dev1's UMEM with the data we want to send and update the frame desc
    let send_frame = &mut dev1_frames[0];
    let data = "Hello, world!".as_bytes();

    println!("sending: {:?}", str::from_utf8(&data).unwrap());

    // Copy the data to the frame
    dev1.umem.copy_data_to_frame(send_frame, &data).unwrap();

    assert_eq!(send_frame.len(), data.len() as u32);

    // 3. Hand over the frame to the kernel for transmission
    assert_eq!(dev1.tx_q.produce_and_wakeup(&dev1_frames[..1]).unwrap(), 1);

    // 4. Read from dev2
    let packets_recvd = dev2
        .rx_q
        .poll_and_consume(&mut dev2_frames[..], 10)
        .unwrap();

    // Check that one of the packets we received matches what we expect.
    for recv_frame in dev2_frames.iter().take(packets_recvd) {
        let frame_ref = dev2.umem.frame_ref(&recv_frame.addr()).unwrap();

        // Check lengths match
        if recv_frame.len() == data.len() as u32 {
            // Check contents match
            if frame_ref[..data.len()] == data[..] {
                println!(
                    "received: {:?}",
                    str::from_utf8(&frame_ref[..data.len()]).unwrap()
                );
                return;
            }
        }
    }

    panic!("no matching frames received")
}

fn main() {
    let (startup_w, mut startup_r) = oneshot::channel();
    let (shutdown_w, shutdown_r) = oneshot::channel();

    let veth_config = VethConfig::new(
        String::from("xsk_ex_dev1"),
        String::from("xsk_ex_dev2"),
        [0xf6, 0xe0, 0xf6, 0xc9, 0x60, 0x0a],
        [0x4a, 0xf1, 0x30, 0xeb, 0x0d, 0x31],
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 1), 24),
        LinkIpAddr::new(Ipv4Addr::new(192, 168, 69, 2), 24),
    );

    let veth_config_clone = veth_config.clone();

    // We'll keep track of ctrl+c events but not let them kill the process
    // immediately as we may need to clean up the veth pair.
    let ctrl_c_events = setup::ctrl_channel().unwrap();

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
            Err(TryRecvError::Closed) => panic!("failed to set up veth pair"),
        }
    }

    // Run example in separate thread so that if it panics we can clean up here
    let (example_done_tx, example_done_rx) = crossbeam_channel::bounded(1);

    let handle = thread::spawn(move || {
        hello_xdp(&veth_config);
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
            // Exit select
        }
    }

    // Delete link
    if let Err(e) = shutdown_w.send(()) {
        eprintln!("veth link thread returned unexpectedly: {:?}", e);
    }

    veth_handle.join().unwrap();
}
