use std::{collections::VecDeque, time::Duration};

mod common;

use common::SocketConfigBuilder;

#[test]
fn tx_queue_produce_no_frames() {
    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    let ((mut umem, _, _), (_socket, mut tx_q, _)) =
        common::build_socket_and_umem_with_retry_on_failure(
            None,
            Some(socket_config),
            3,
            Duration::from_millis(1000),
        )
        .unwrap();

    let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

    assert_eq!(tx_q.produce(&mut frame_descs, 0), 0);
}

#[test]
fn tx_queue_produce_lt_tx_size_frames() {
    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    let ((mut umem, _, _), (_socket, mut tx_q, _)) =
        common::build_socket_and_umem_with_retry_on_failure(
            None,
            Some(socket_config),
            3,
            Duration::from_millis(1000),
        )
        .unwrap();

    let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

    assert_eq!(tx_q.produce(&mut frame_descs, 3), 3);
}

#[test]
fn tx_queue_produce_eq_tx_size_frames() {
    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    let ((mut umem, _, _), (_socket, mut tx_q, _)) =
        common::build_socket_and_umem_with_retry_on_failure(
            None,
            Some(socket_config),
            3,
            Duration::from_millis(1000),
        )
        .unwrap();

    let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

    assert_eq!(tx_q.produce(&mut frame_descs, 4), 4);
}

#[test]
fn tx_queue_produce_gt_tx_size_frames() {
    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    let ((mut umem, _, _), (_socket, mut tx_q, _)) =
        common::build_socket_and_umem_with_retry_on_failure(
            None,
            Some(socket_config),
            3,
            Duration::from_millis(1000),
        )
        .unwrap();

    let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

    assert_eq!(tx_q.produce(&mut frame_descs, 5), 4);
}

#[test]
fn tx_queue_produce_frames_until_none_accepted() {
    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    let ((mut umem, _, _), (_socket, mut tx_q, _)) =
        common::build_socket_and_umem_with_retry_on_failure(
            None,
            Some(socket_config),
            3,
            Duration::from_millis(1000),
        )
        .unwrap();

    let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

    assert_eq!(tx_q.produce(&mut frame_descs, 3), 3);

    assert_eq!(tx_q.produce(&mut frame_descs, 2), 1);

    assert_eq!(tx_q.produce(&mut frame_descs, 1), 0);
}

#[test]
fn tx_queue_produce_and_wakeup() {
    let socket_config = SocketConfigBuilder {
        tx_queue_size: 4,
        ..SocketConfigBuilder::default()
    }
    .build();

    let ((mut umem, _, _), (_socket, mut tx_q, _)) =
        common::build_socket_and_umem_with_retry_on_failure(
            None,
            Some(socket_config),
            3,
            Duration::from_millis(1000),
        )
        .unwrap();

    let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

    let cnt = tx_q
        .produce_and_wakeup(&mut frame_descs, 4)
        .expect("Poll error");

    assert_eq!(cnt, 4);
}
