use std::sync::{Arc, Weak};

use bytes::Bytes;
use tokio::sync::{mpsc, OwnedSemaphorePermit, RwLock, Semaphore};
use webrtc::data_channel::{
    data_channel_init::RTCDataChannelInit, data_channel_message::DataChannelMessage, RTCDataChannel,
};
use webrtc::peer_connection::RTCPeerConnection;

pub const CTRL_REL_LABEL: &str = "ctrl_rel";
pub const CTRL_RT_LABEL: &str = "ctrl_rt";
pub const BULK_LABEL: &str = "bulk";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlLane {
    Reliable,
    Realtime,
    Bulk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlChannelInfo {
    pub label: String,
    pub ordered: bool,
    pub max_retransmits: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlChannels {
    pub reliable: ControlChannelInfo,
    pub realtime: ControlChannelInfo,
    pub bulk: ControlChannelInfo,
}

#[derive(Debug)]
pub(crate) struct QueuedBytes {
    bytes: Bytes,
    _byte_permit: OwnedSemaphorePermit,
}

impl QueuedBytes {
    pub(crate) fn into_bytes(self) -> Bytes {
        self.bytes
    }
}

pub(crate) struct ControlState {
    pub(crate) reliable: RwLock<Option<Arc<RTCDataChannel>>>,
    pub(crate) realtime: RwLock<Option<Arc<RTCDataChannel>>>,
    pub(crate) bulk: RwLock<Option<Arc<RTCDataChannel>>>,
    reliable_tx: mpsc::Sender<QueuedBytes>,
    realtime_tx: mpsc::Sender<QueuedBytes>,
    bulk_tx: mpsc::Sender<QueuedBytes>,
    reliable_budget: Arc<Semaphore>,
    realtime_budget: Arc<Semaphore>,
    bulk_budget: Arc<Semaphore>,
}

impl ControlState {
    pub(crate) fn new(
        capacity: usize,
        reliable_queue_bytes: usize,
        realtime_queue_bytes: usize,
        bulk_queue_bytes: usize,
    ) -> (
        Arc<Self>,
        mpsc::Receiver<QueuedBytes>,
        mpsc::Receiver<QueuedBytes>,
        mpsc::Receiver<QueuedBytes>,
    ) {
        let (reliable_tx, reliable_rx) = mpsc::channel(capacity);
        let (realtime_tx, realtime_rx) = mpsc::channel(capacity);
        let (bulk_tx, bulk_rx) = mpsc::channel(capacity);
        (
            Arc::new(Self {
                reliable: RwLock::new(None),
                realtime: RwLock::new(None),
                bulk: RwLock::new(None),
                reliable_tx,
                realtime_tx,
                bulk_tx,
                reliable_budget: Arc::new(Semaphore::new(reliable_queue_bytes)),
                realtime_budget: Arc::new(Semaphore::new(realtime_queue_bytes)),
                bulk_budget: Arc::new(Semaphore::new(bulk_queue_bytes)),
            }),
            reliable_rx,
            realtime_rx,
            bulk_rx,
        )
    }

    pub(crate) async fn install(
        self: &Arc<Self>,
        channel: Arc<RTCDataChannel>,
        failure_pc: Weak<RTCPeerConnection>,
    ) -> Result<(), crate::TransportError> {
        let lane = match channel.label() {
            CTRL_REL_LABEL => ControlLane::Reliable,
            CTRL_RT_LABEL => ControlLane::Realtime,
            BULK_LABEL => ControlLane::Bulk,
            label => {
                let label = label.to_owned();
                let _ = channel.close().await;
                return Err(crate::TransportError::Message(format!(
                    "unexpected WebRTC data channel label {label}"
                )));
            }
        };
        validate_channel_semantics(lane, &channel)?;
        let slot = match lane {
            ControlLane::Reliable => &self.reliable,
            ControlLane::Realtime => &self.realtime,
            ControlLane::Bulk => &self.bulk,
        };
        let mut installed = slot.write().await;
        if installed.is_some() {
            let _ = channel.close().await;
            return Err(crate::TransportError::Message(format!(
                "duplicate WebRTC {lane:?} data channel"
            )));
        }
        let tx = match lane {
            ControlLane::Reliable => self.reliable_tx.clone(),
            ControlLane::Realtime => self.realtime_tx.clone(),
            ControlLane::Bulk => self.bulk_tx.clone(),
        };
        let budget = match lane {
            ControlLane::Reliable => Arc::clone(&self.reliable_budget),
            ControlLane::Realtime => Arc::clone(&self.realtime_budget),
            ControlLane::Bulk => Arc::clone(&self.bulk_budget),
        };
        let overflow_channel = weak_callback_owner(&channel);
        channel.on_message(Box::new(move |message: DataChannelMessage| {
            let tx = tx.clone();
            let budget = Arc::clone(&budget);
            let overflow_channel = overflow_channel.clone();
            let failure_pc = failure_pc.clone();
            Box::pin(async move {
                let permits = match try_reserve_bytes(budget, message.data.len()) {
                    Some(permits) => permits,
                    None => {
                        if let Some(overflow_channel) = overflow_channel.upgrade() {
                            let _ = overflow_channel.close().await;
                        }
                        if let Some(failure_pc) = failure_pc.upgrade() {
                            let _ = failure_pc.close().await;
                        }
                        return;
                    }
                };
                if tx
                    .try_send(QueuedBytes {
                        bytes: message.data,
                        _byte_permit: permits,
                    })
                    .is_err()
                {
                    if let Some(overflow_channel) = overflow_channel.upgrade() {
                        let _ = overflow_channel.close().await;
                    }
                    if let Some(failure_pc) = failure_pc.upgrade() {
                        let _ = failure_pc.close().await;
                    }
                }
            })
        }));
        *installed = Some(channel);
        Ok(())
    }

    pub(crate) async fn channel(&self, lane: ControlLane) -> Option<Arc<RTCDataChannel>> {
        match lane {
            ControlLane::Reliable => self.reliable.read().await.clone(),
            ControlLane::Realtime => self.realtime.read().await.clone(),
            ControlLane::Bulk => self.bulk.read().await.clone(),
        }
    }
}

pub(crate) fn weak_callback_owner<T>(owner: &Arc<T>) -> Weak<T> {
    Arc::downgrade(owner)
}

fn try_reserve_bytes(budget: Arc<Semaphore>, bytes: usize) -> Option<OwnedSemaphorePermit> {
    u32::try_from(bytes)
        .ok()
        .and_then(|bytes| budget.try_acquire_many_owned(bytes).ok())
}

fn validate_channel_semantics(
    lane: ControlLane,
    channel: &RTCDataChannel,
) -> Result<(), crate::TransportError> {
    let valid = match lane {
        ControlLane::Reliable | ControlLane::Bulk => {
            channel.ordered()
                && channel.max_retransmits().is_none()
                && channel.max_packet_lifetime().is_none()
        }
        ControlLane::Realtime => {
            !channel.ordered()
                && channel.max_retransmits() == Some(0)
                && channel.max_packet_lifetime().is_none()
        }
    };
    if valid {
        Ok(())
    } else {
        Err(crate::TransportError::Message(format!(
            "WebRTC {lane:?} data channel has incompatible reliability semantics"
        )))
    }
}

pub(crate) fn reliable_channel_init() -> RTCDataChannelInit {
    RTCDataChannelInit {
        ordered: Some(true),
        ..Default::default()
    }
}

pub(crate) fn realtime_channel_init() -> RTCDataChannelInit {
    RTCDataChannelInit {
        ordered: Some(false),
        max_retransmits: Some(0),
        ..Default::default()
    }
}

pub(crate) fn channel_info(channel: &RTCDataChannel) -> ControlChannelInfo {
    ControlChannelInfo {
        label: channel.label().to_owned(),
        ordered: channel.ordered(),
        max_retransmits: channel.max_retransmits(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingress_budget_is_bounded_by_retained_bytes() {
        let budget = Arc::new(Semaphore::new(8));
        let retained = try_reserve_bytes(Arc::clone(&budget), 6).expect("reserve six bytes");
        assert!(try_reserve_bytes(Arc::clone(&budget), 3).is_none());
        drop(retained);
        assert!(try_reserve_bytes(budget, 8).is_some());
    }

    #[test]
    fn callback_reference_does_not_retain_pc_or_channel_owner() {
        let owner = Arc::new(());
        let failure_owner = weak_callback_owner(&owner);

        assert_eq!(Arc::strong_count(&owner), 1);
        drop(owner);
        assert!(failure_owner.upgrade().is_none());
    }
}
