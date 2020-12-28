use std::{collections::VecDeque, io};

use crate::{CompQueue, DataError, FillQueue, FrameDesc, RxQueue, TxQueue, Umem};

struct Xsk<'umem> {
    tx_q: TxQueue<'umem>,
    rx_q: RxQueue<'umem>,
    fill_q: FillQueue<'umem>,
    comp_q: CompQueue<'umem>,
    umem: Umem<'umem>,
}

pub struct FrameManager<'umem> {
    free_frames: Vec<FrameDesc<'umem>>,
    tx_pending_submission: Vec<FrameDesc<'umem>>,
    tx_pending_completion: VecDeque<FrameDesc<'umem>>,
    rx_fill_pending: VecDeque<FrameDesc<'umem>>,
    rx_fill_completed: VecDeque<FrameDesc<'umem>>,
    xsk: Xsk<'umem>,
}

impl<'umem> FrameManager<'umem> {
    /// The number of free frames available to be used for either sending or receiving packets.
    pub fn num_free_frames(&self) -> usize {
        self.free_frames.len()
    }

    /// Try and copy `data` into the next available UMEM frame. Returns `None` if
    /// there are no free frames available to write to, otherwise will return `Some`
    /// to indicate that copying was successful.
    pub fn write_to_next_available_frame(&mut self, data: &[u8]) -> Result<Option<()>, DataError> {
        if data.len() == 0 {
            return Ok(Some(()));
        }

        if let Err(e) = self.xsk.umem.check_data_valid(data) {
            return Err(e);
        }

        match self.free_frames.pop() {
            None => Ok(None),
            Some(mut frame_desc) => {
                unsafe {
                    self.xsk
                        .umem
                        .copy_data_to_frame_unchecked(&mut frame_desc, data)
                };

                self.tx_pending_submission.push(frame_desc);

                Ok(Some(()))
            }
        }
    }

    /// Submit all umem frames that have been written to for transmission.
    /// Blocks until all frames have been submitted.
    pub fn submit_all_pending_tx(&mut self) -> io::Result<usize> {
        let nb = self.tx_pending_submission.len();

        if nb == 0 {
            return Ok(0);
        }

        while self
            .xsk
            .tx_q
            .produce_and_wakeup(&self.tx_pending_submission[..])?
            != nb
        {
            // Keep trying until all frames submitted
        }

        while let Some(frame_desc) = self.tx_pending_submission.pop() {
            self.tx_pending_completion.push_back(frame_desc)
        }

        Ok(nb)
    }

    /// Clear the completion ring of any frames that have finished transmitting.
    pub fn clear_any_completed_tx(&mut self) -> usize {
        if self.tx_pending_completion.len() == 0 {
            return 0;
        }

        let nb = self
            .xsk
            .comp_q
            .consume(&mut self.tx_pending_completion.make_contiguous()[..]);

        for _ in 0..nb {
            let frame_desc = self
                .tx_pending_completion
                .pop_front()
                .expect("frame_desc present as nb <= num frames pending completion");

            self.free_frames.push(frame_desc);
        }

        nb
    }

    /// Try submit `nb` frames to the fill ring to be used for receiving packets.
    /// Returns `None` if not enough free frames currently available to add, otherwise
    /// returns `Some` to indicate that `nb` frames were added to the fill ring.
    pub fn submit_frames_for_rx(&mut self, nb: usize) -> io::Result<Option<()>> {
        let nfree = self.num_free_frames();

        if nb > nfree {
            return Ok(None);
        }

        let offset = nfree - nb;

        while self.xsk.fill_q.produce_and_wakeup(
            &mut self.free_frames[offset..],
            self.xsk.rx_q.fd(),
            100,
        )? != nb
        {
            // Keep trying until `nb` frames added
        }

        for _ in 0..offset {
            let frame_desc = self
                .free_frames
                .pop()
                .expect("frame_desc present as nb <= num free frames");

            self.rx_fill_pending.push_back(frame_desc);
        }

        Ok(Some(()))
    }

    /// Check the fill ring for received packets and add them to the rx ring to be read.
    /// Returns the number of new frames able to be read.
    pub fn check_for_completed_rx(&mut self) -> io::Result<usize> {
        if self.rx_fill_pending.len() == 0 {
            return Ok(0);
        }

        let nb = self
            .xsk
            .rx_q
            .poll_and_consume(&mut self.rx_fill_pending.make_contiguous()[..], 100)?;

        for _ in 0..nb {
            let frame_desc = self
                .rx_fill_pending
                .pop_front()
                .expect("frame_desc present as nb <= num rx frames free");

            self.rx_fill_completed.push_back(frame_desc);
        }

        Ok(nb)
    }

    /// Apply `reader` to the next readable frame in the completion ring.
    /// Once `reader` is done, the contents of the read frame will be discarded.
    /// `None` will be passed to the function if there are no frames to read.
    pub fn read_from_next_available_frame<F>(&mut self, reader: &mut F)
    where
        F: FnMut(Option<&[u8]>),
    {
        match self.rx_fill_completed.pop_front() {
            None => reader(None),
            Some(frame_desc) => {
                let data = Some(unsafe {
                    self.xsk
                        .umem
                        .frame_ref_at_addr_unchecked(&frame_desc.addr())
                });

                self.free_frames.push(frame_desc);

                reader(data);
            }
        }
    }
}
