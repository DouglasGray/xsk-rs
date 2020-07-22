pub mod poll;
pub mod socket;
pub mod umem;

pub(crate) fn get_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

#[cfg(test)]
mod tests {
    //use super::*;
    //#[test]
    // fn check_new_mmap_works() {
    //     let config = MmapAreaConfig {
    //         frame_count: 256,
    //         frame_size: 2048,
    //         use_huge_pages: false,
    //     };

    //     MmapArea::new(&config).expect("Creating memory mapped area failed");
    // }

    //#[test]
    // fn check_new_umem_works() {
    //     let mmap_config = MmapAreaConfig {
    //         frame_count: 256,
    //         frame_size: 2048,
    //         use_huge_pages: false,
    //     };

    //     let mmap_area = MmapArea::new(&mmap_config).expect("Creating memory mapped area failed");

    //     let umem_config = UmemConfig {
    //         fill_queue_size: 2048,
    //         comp_queue_size: 2048,
    //         frame_headroom: 0,
    //     };

    //     Umem::new(&umem_config, mmap_area).expect("Initialisation of UMEM failed");
    // }

    // #[test]
    // fn check_new_xsk_socket_works() {
    //     let mmap_config = MmapAreaConfig {
    //         frame_count: 256,
    //         frame_size: 2048,
    //         use_huge_pages: false,
    //     };

    //     let mmap_area = MmapArea::new(mmap_config).expect("Creating mmap-ed area failed");

    //     let umem_config = UmemConfig {
    //         fill_queue_size: 2048,
    //         comp_queue_size: 2048,
    //         frame_headroom: 0,
    //     };

    //     let (mut umem, _fq, _cq) =
    //         Umem::new(umem_config, &mmap_area).expect("Initialisation of UMEM failed");

    //     let socket_config = SocketConfig {
    //         rx_queue_size: 2048,
    //         tx_queue_size: 2048,
    //     };

    //     Socket::new("lo", 0, &mut umem, socket_config).expect("Failed to create and bind socket");
    // }

    // #[test]
    // fn check_needs_wakeup_flag_affects_tx_q_and_fill_q() {
    //     let mmap_config = MmapAreaConfig {
    //         frame_count: 256,
    //         frame_size: 2048,
    //         use_huge_pages: false,
    //     };

    //     let mmap_area = MmapArea::new(&mmap_config).expect("Creating memory mapped area failed");

    //     let umem_config = UmemConfig {
    //         fill_queue_size: 2048,
    //         comp_queue_size: 2048,
    //         frame_headroom: 0,
    //     };

    //     let (mut umem, fq, _cq) =
    //         Umem::new(&umem_config, mmap_area).expect("Initialisation of UMEM failed");

    //     let socket_config = SocketConfig {
    //         rx_queue_size: 2048,
    //         tx_queue_size: 2048,
    //     };

    //     let (_socket, tx_q, _rx_q) = Socket::new("lo", 0, &mut umem, &socket_config)
    //         .expect("Failed to create and bind socket");

    //     assert!(fq.needs_wakeup());
    //     assert!(tx_q.needs_wakeup());
    // }
}
