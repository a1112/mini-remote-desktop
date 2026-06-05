use mrd_transport_quic_quinn::{QuicAuFrame, QuicMediaFrame};
use std::collections::BTreeMap;

pub(crate) trait LanOrderedMediaFrame {
    fn frame_id(&self) -> u32;
}

impl LanOrderedMediaFrame for QuicAuFrame {
    fn frame_id(&self) -> u32 {
        self.frame_id
    }
}

impl LanOrderedMediaFrame for QuicMediaFrame {
    fn frame_id(&self) -> u32 {
        self.frame_id
    }
}

pub(crate) struct LanMediaFrameOrderer<T = QuicAuFrame> {
    next_frame_id: Option<u32>,
    max_pending_frames: usize,
    pending: BTreeMap<u32, T>,
}

impl<T: LanOrderedMediaFrame> LanMediaFrameOrderer<T> {
    pub(crate) fn new(max_pending_frames: usize) -> Self {
        Self {
            next_frame_id: None,
            max_pending_frames: max_pending_frames.max(1),
            pending: BTreeMap::new(),
        }
    }

    pub(crate) fn push(&mut self, frame: T) -> Vec<T> {
        let frame_id = frame.frame_id();
        if self
            .next_frame_id
            .is_some_and(|next_frame_id| frame_id < next_frame_id)
        {
            return Vec::new();
        }

        self.next_frame_id.get_or_insert(frame_id);
        self.pending.entry(frame_id).or_insert(frame);

        let mut ready = self.drain_contiguous();
        if ready.is_empty() && self.pending.len() >= self.max_pending_frames {
            if let Some(next_frame_id) = self.pending.keys().next().copied() {
                self.next_frame_id = Some(next_frame_id);
                ready = self.drain_contiguous();
            }
        }
        ready
    }

    fn drain_contiguous(&mut self) -> Vec<T> {
        let mut ready = Vec::new();
        while let Some(next_frame_id) = self.next_frame_id {
            let Some(frame) = self.pending.remove(&next_frame_id) else {
                break;
            };
            self.next_frame_id = Some(next_frame_id.wrapping_add(1));
            ready.push(frame);
        }
        ready
    }
}
