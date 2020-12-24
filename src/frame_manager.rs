use std::{collections::VecDeque, io};

use crate::{CompQueue, FillQueue, FrameDesc, RxQueue, TxQueue, Umem, UmemAccessError};

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
    free_frames: Vec<FrameDesc>,
    tx_pending_submission: Vec<FrameDesc>,
    tx_pending_completion: Vec<FrameDesc>,
    rx_pending: Vec<FrameDesc>,
    rx_completed: VecDeque<FrameDesc>,
    xsk: Xsk<'umem>,
}

impl<'umem> FrameManager<'umem> {
    /// The number free frames available to be used for either sending or receiving packets.
    pub fn free_frames(&self) -> usize {
        self.free_frames.len()
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

        match self.free_frames.pop() {
            None => None,
            Some(frame_desc) => {
                let res = unsafe {
                    self.xsk
                        .umem
                        .copy_data_to_frame_at_addr(&frame_desc.addr(), data)
                };

                match res {
                    Ok(_) => {
                        self.tx_pending_submission.push(frame_desc);
                    }
                    Err(_) => {
                        // Write failed to add back to queue
                        self.free_frames.push(frame_desc);
                    }
                };

                Some(res)
            }
        }
    }

    /// Submit all umem frames that have been written to for transmission.
    /// Blocks until all frames have been submitted.
    pub fn submit_pending_tx(&mut self) -> io::Result<usize> {
        let nb = self.tx_pending_submission.len();

        if nb == 0 {
            return Ok(0);
        }

        if self.use_need_wakeup {
            while self
                .xsk
                .tx_q
                .produce_and_wakeup(&self.tx_pending_submission[..])?
                != nb
            {
                // Keep trying until all frames submitted
            }
        } else {
            while self.xsk.tx_q.produce(&self.tx_pending_submission[..]) != nb {
                // Keep trying until all frames submitted
            }
        }

        self.tx_pending_completion
            .append(&mut self.tx_pending_submission);

        Ok(nb)
    }

    /// Check the completion ring for any frames that have finished transmitting.
    pub fn check_for_completed_tx(&mut self) -> usize {
        if self.tx_pending_completion.len() == 0 {
            return 0;
        }

        let nb = self.xsk.comp_q.consume(&mut self.tx_pending_completion[..]);

        if nb > 0 {
            // `nb` returns the number of frames consumed from the completion ring,
            // the details of which will be written to the first `nb` elements
            // of `self.tx_pending_completion`. Splitting at `nb` therefore leaves
            // the written-to frames in `self.tx_pending_completion` and returns the
            // unwritten-to lot.
            let unwritten_frames = self.tx_pending_completion.split_off(nb);

            self.free_frames.append(&mut self.tx_pending_completion);

            self.tx_pending_completion = unwritten_frames;
        }

        nb
    }

    /// Apply `reader` to the next readable frame in the queue. Once `reader` is done, the
    /// contents of the read frame will be discarded.
    /// `None` will be passed to the function if there are no frames to read.
    pub fn read_from_next_available_frame<F>(&mut self, reader: &mut F)
    where
        F: FnMut(Option<Result<&[u8], UmemAccessError>>),
    {
        match self.rx_completed.pop_front() {
            None => reader(None),
            Some(frame_desc) => {
                let data = Some(unsafe { self.xsk.umem.frame_ref_at_addr(&frame_desc.addr()) });

                self.free_frames.push(frame_desc);

                reader(data);
            }
        }
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
