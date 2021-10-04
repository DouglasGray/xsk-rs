//! Re-exports the key types.

pub use super::{
    config::{FrameSize, Interface, QueueSize, SocketConfig, UmemConfig},
    socket::{RxQueue, Socket, TxQueue},
    umem::{frame::Frame, CompQueue, FillQueue, Umem},
};
