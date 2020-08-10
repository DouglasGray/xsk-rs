// use std::{collections::VecDeque, time::Duration};

// use rust_xsk::poll::Milliseconds;

// mod common;

// use common::UmemConfigBuilder;

// #[test]
// fn fill_queue_produce_no_frames() {
//     let umem_config = UmemConfigBuilder {
//         fill_queue_size: 4,
//         ..UmemConfigBuilder::default()
//     }
//     .build();

//     let (mut umem, mut fill_q, _comp_q) = common::build_umem(Some(umem_config.clone()));

//     let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

//     assert_eq!(fill_q.produce(&mut frame_descs, 0), 0);
// }

// #[test]
// fn fill_queue_produce_lt_fill_size_frames() {
//     let umem_config = UmemConfigBuilder {
//         fill_queue_size: 4,
//         ..UmemConfigBuilder::default()
//     }
//     .build();

//     let (mut umem, mut fill_q, _comp_q) = common::build_umem(Some(umem_config.clone()));

//     let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

//     assert_eq!(fill_q.produce(&mut frame_descs, 3), 3);
// }

// #[test]
// fn fill_queue_produce_eq_fill_size_frames() {
//     let umem_config = UmemConfigBuilder {
//         fill_queue_size: 4,
//         ..UmemConfigBuilder::default()
//     }
//     .build();

//     let (mut umem, mut fill_q, _comp_q) = common::build_umem(Some(umem_config.clone()));

//     let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

//     assert_eq!(fill_q.produce(&mut frame_descs, 4), 4);
// }

// #[test]
// fn fill_queue_produce_gt_fill_size_frames() {
//     let umem_config = UmemConfigBuilder {
//         fill_queue_size: 4,
//         ..UmemConfigBuilder::default()
//     }
//     .build();

//     let (mut umem, mut fill_q, _comp_q) = common::build_umem(Some(umem_config.clone()));

//     let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

//     assert_eq!(fill_q.produce(&mut frame_descs, 5), 4);
// }

// #[test]
// fn fill_queue_produce_frames_until_none_accepted() {
//     let umem_config = UmemConfigBuilder {
//         fill_queue_size: 4,
//         ..UmemConfigBuilder::default()
//     }
//     .build();

//     let (mut umem, mut fill_q, _comp_q) = common::build_umem(Some(umem_config.clone()));

//     let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

//     assert_eq!(fill_q.produce(&mut frame_descs, 3), 3);

//     assert_eq!(fill_q.produce(&mut frame_descs, 2), 1);

//     assert_eq!(fill_q.produce(&mut frame_descs, 1), 0);
// }

// #[test]
// fn fill_queue_produce_and_wakeup() {
//     let umem_config = UmemConfigBuilder {
//         fill_queue_size: 4,
//         ..UmemConfigBuilder::default()
//     }
//     .build();

//     let ((mut umem, mut fill_q, _), (socket, _, _)) =
//         common::build_socket_and_umem_with_retry_on_failure(
//             Some(umem_config),
//             None,
//             3,
//             Duration::from_millis(100),
//         )
//         .unwrap();

//     let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

//     let timeout = Milliseconds::new(1000);

//     let cnt = fill_q
//         .produce_and_wakeup(&mut frame_descs, 4, socket.file_descriptor(), &timeout)
//         .expect("Poll error");

//     assert_eq!(cnt, 4);
// }
