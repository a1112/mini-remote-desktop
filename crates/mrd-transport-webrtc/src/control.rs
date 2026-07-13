use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::{mpsc, RwLock};
use webrtc::data_channel::{
    data_channel_init::RTCDataChannelInit, data_channel_message::DataChannelMessage, RTCDataChannel,
};

pub const CTRL_REL_LABEL: &str = "ctrl_rel";
pub const CTRL_RT_LABEL: &str = "ctrl_rt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlLane {
    Reliable,
    Realtime,
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
}

pub(crate) struct ControlState {
    pub(crate) reliable: RwLock<Option<Arc<RTCDataChannel>>>,
    pub(crate) realtime: RwLock<Option<Arc<RTCDataChannel>>>,
    reliable_tx: mpsc::Sender<Bytes>,
    realtime_tx: mpsc::Sender<Bytes>,
}

impl ControlState {
    pub(crate) fn new(
        capacity: usize,
    ) -> (Arc<Self>, mpsc::Receiver<Bytes>, mpsc::Receiver<Bytes>) {
        let (reliable_tx, reliable_rx) = mpsc::channel(capacity);
        let (realtime_tx, realtime_rx) = mpsc::channel(capacity);
        (
            Arc::new(Self {
                reliable: RwLock::new(None),
                realtime: RwLock::new(None),
                reliable_tx,
                realtime_tx,
            }),
            reliable_rx,
            realtime_rx,
        )
    }

    pub(crate) async fn install(self: &Arc<Self>, channel: Arc<RTCDataChannel>) {
        let lane = match channel.label() {
            CTRL_REL_LABEL => ControlLane::Reliable,
            CTRL_RT_LABEL => ControlLane::Realtime,
            _ => return,
        };
        let tx = match lane {
            ControlLane::Reliable => self.reliable_tx.clone(),
            ControlLane::Realtime => self.realtime_tx.clone(),
        };
        channel.on_message(Box::new(move |message: DataChannelMessage| {
            let tx = tx.clone();
            Box::pin(async move {
                let _ = tx.send(message.data).await;
            })
        }));
        match lane {
            ControlLane::Reliable => *self.reliable.write().await = Some(channel),
            ControlLane::Realtime => *self.realtime.write().await = Some(channel),
        }
    }

    pub(crate) async fn channel(&self, lane: ControlLane) -> Option<Arc<RTCDataChannel>> {
        match lane {
            ControlLane::Reliable => self.reliable.read().await.clone(),
            ControlLane::Realtime => self.realtime.read().await.clone(),
        }
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
