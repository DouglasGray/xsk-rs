use std::{num::NonZeroU32, str, thread};
use tokio::{
    runtime::Runtime,
    sync::oneshot::{self, error::TryRecvError},
};
use xsk_rs::{FillQueue, FrameDesc, RxQueue, Socket, SocketConfig, TxQueue, Umem, UmemConfig};

mod setup;

struct SocketState<'umem> {
    umem: Umem<'umem>,
    fill_q: FillQueue<'umem>,
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
    let (mut umem, fill_q, _comp_q, frame_descs) = Umem::builder(umem_config)
        .create_mmap()
        .expect(format!("Failed to create mmap area for {}", if_name).as_str())
        .create_umem()
        .expect(format!("Failed to create umem for {}", if_name).as_str());

    let (tx_q, rx_q) = Socket::new(socket_config, &mut umem, if_name, queue_id)
        .expect(format!("Failed to build socket for {}", if_name).as_str());

    SocketState {
        umem,
        fill_q,
        tx_q,
        rx_q,
        frame_descs,
    }
}

fn hello_xdp(dev1_if_name: String, dev2_if_name: String) {
    // Create umem and socket configs
    let umem_config = UmemConfig::default(NonZeroU32::new(16).unwrap(), false);
    let socket_config = SocketConfig::default();

    let mut dev1 =
        build_socket_and_umem(umem_config.clone(), socket_config.clone(), &dev1_if_name, 0);

    let mut dev2 = build_socket_and_umem(umem_config, socket_config, &dev2_if_name, 0);

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

    println!("Sending: {:?}", str::from_utf8(&data).unwrap());

    // Copy the data to the frame
    assert_eq!(
        dev1.umem
            .copy_data_to_frame(&send_frame.addr(), &data)
            .unwrap(),
        data.len()
    );

    // Update the frame descriptor's length
    send_frame.set_len(data.len() as u32);

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
                    "Received: {:?}",
                    str::from_utf8(&frame_ref[..data.len()]).unwrap()
                );
                return;
            }
        }
    }

    panic!("No matching packets received")
}

fn main() {
    let dev1_if_name = String::from("xsk_ex_dev1");
    let dev2_if_name = String::from("xsk_ex_dev2");

    let (startup_w, mut startup_r) = oneshot::channel();
    let (shutdown_w, shutdown_r) = oneshot::channel();

    let dev1_if_name_clone = dev1_if_name.clone();
    let dev2_if_name_clone = dev2_if_name.clone();

    let veth_handle = thread::spawn(move || {
        let mut runtime = Runtime::new().unwrap();

        runtime.block_on(setup::run_veth_link(
            &dev1_if_name_clone,
            &dev2_if_name_clone,
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

    // Run example in separate thread so that if it panics we can clean up here
    let ex_handle = thread::spawn(move || hello_xdp(dev1_if_name, dev2_if_name));

    let res = ex_handle.join();

    // Tell link to close
    if let Err(e) = shutdown_w.send(()) {
        eprintln!("veth link thread returned unexpectedly: {:?}", e);
    }

    veth_handle.join().unwrap();

    res.unwrap();
}
