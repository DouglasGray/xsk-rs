use std::{collections::VecDeque, time::Duration};

mod common;

use common::SocketConfigBuilder;

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
            Duration::from_millis(100),
        )
        .unwrap();

    let mut frame_descs = VecDeque::from(umem.consume_frame_descs().unwrap());

    assert_eq!(tx_q.produce(&mut frame_descs, 3), 3);
}
