use std::time::{Duration, Instant};

use crate::file_ops::service::FileOpService;
use crate::webdav_client::model::WebDavEndpoint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountState {
    Init,
    Opening,
    Mounted,
    Degraded,
    Closing,
    Closed,
    Error,
}

#[derive(Debug, Clone)]
pub struct MountSession {
    pub mount_id: u64,
    pub endpoint: WebDavEndpoint,
    pub flags: u32,
    pub state: MountState,
    pub last_heartbeat_at: Instant,
    pub file_service: FileOpService,
}

impl MountSession {
    pub fn new(
        mount_id: u64,
        endpoint: WebDavEndpoint,
        flags: u32,
        file_service: FileOpService,
    ) -> Self {
        Self {
            mount_id,
            endpoint,
            flags,
            state: MountState::Init,
            last_heartbeat_at: Instant::now(),
            file_service,
        }
    }

    pub fn open(&mut self) {
        self.state = MountState::Opening;
        self.state = MountState::Mounted;
    }

    pub fn heartbeat(&mut self) {
        self.last_heartbeat_at = Instant::now();
        if self.state == MountState::Degraded {
            self.state = MountState::Mounted;
        }
    }

    pub fn close(&mut self) {
        self.state = MountState::Closing;
        self.state = MountState::Closed;
    }

    pub fn apply_timeout(&mut self, now: Instant, timeout: Duration) {
        if now.saturating_duration_since(self.last_heartbeat_at) > timeout
            && self.state == MountState::Mounted
        {
            self.state = MountState::Degraded;
        }
    }
}
