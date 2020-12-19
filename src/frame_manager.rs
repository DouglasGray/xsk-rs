use std::io;

use crate::{CompQueue, FillQueue, FrameDesc, RxQueue, TxQueue, Umem, UmemAccessError};

pub enum TxError {
    NoFreeFrames,
    UmemAccessError(UmemAccessError),
    IoError(io::Error),
}

pub enum RxError {
    NoFreeFrames,
    IoError(io::Error),
}

struct Queue {
    frames: Vec<FrameDesc>,
    occupied: usize,
}

struct Xsk<'umem> {
    tx_q: TxQueue<'umem>,
    rx_q: RxQueue<'umem>,
    fill_q: FillQueue<'umem>,
    comp_q: CompQueue<'umem>,
    frame_descs: Vec<FrameDesc>,
    umem: Umem<'umem>,
}

pub struct FrameManager<'umem> {
    use_need_wakeup: bool,
    free_frames: Queue,
    tx_pending_completion: Queue,
    tx_pending_submission: Queue,
    rx_pending: Queue,
    rx_completed: Queue,
    xsk: Xsk<'umem>,
}

impl<'umem> FrameManager<'umem> {
    /// The number free frames available to be used for either sending or receiving packets.
    pub fn free_frames(&self) -> usize {
        self.free_frames.occupied
    }

    /// Try and copy `data` into the next available UMEM frame. Returns `None` if
    /// there are no free frames available to write to.
    pub fn write_to_next_available_frame(
        &mut self,
        data: &[u8],
    ) -> Option<Result<usize, UmemAccessError>> {
        if data.len() == 0 {
            return Some(Ok(0));
        }
        if self.free_frames() == 0 {
            return None;
        }

        // Take a free frame, copy the data to UMEM and then move that frame to the
        // pending tx queue.

        unimplemented!()
    }

    /// Submit umem frames that have been written to for transmission.
    pub fn submit_pending_tx(&mut self) -> io::Result<usize> {
        unimplemented!()
    }

    /// Check the completion ring for any frames that have finished transmitting.
    pub fn check_for_completed_tx(&mut self) -> usize {
        unimplemented!()
    }

    /// Return frames which have data ready to read
    pub fn read_from_next_available_frame<F>(&mut self, reader: &mut F)
    where
        F: FnMut(&[u8]) -> (),
    {
        let frame_desc = &self.rx_completed.frames[self.rx_completed.occupied];

        let data_ref = unsafe { self.xsk.umem.frame_ref_unchecked(&frame_desc.addr()) };

        reader(&data_ref[..frame_desc.len()]);

        // Add to free frames and decrement rx_completed
    }

    /// Submit a given number of frames to the fill ring, to be used for receiving packets.
    pub fn submit_frames_for_rx(&mut self, nb: usize) -> Option<io::Result<usize>> {
        if nb > self.free_frames() {
            return None;
        }

        unimplemented!()
    }

    /// Check the rx ring for received packets
    pub fn check_for_completed_rx(&mut self) -> usize {
        unimplemented!()
    }

    pub fn clear_completed_rx(&mut self) -> usize {
        unimplemented!()
    }
}
