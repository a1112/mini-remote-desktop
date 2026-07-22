//! In-memory transport mux used to exercise the adapter contract.

use std::sync::{Arc, Weak};

use anyhow::Result;
use mrd_application::ports::{
    TransportEnvelope, TransportLane, TransportMuxPort, TransportRouteKind, TransportRouteSnapshot,
    TransportSendOutcome,
};
use mrd_proto::SessionId;

use super::{SessionMuxCore, TransportMuxConfig};

/// One endpoint in a paired in-memory transport mux.
#[derive(Debug)]
pub struct MemoryTransportMux {
    core: Arc<SessionMuxCore>,
    peer: Weak<SessionMuxCore>,
}

impl MemoryTransportMux {
    /// Create two connected in-memory endpoints for one session.
    pub fn pair(session_id: SessionId, config: TransportMuxConfig) -> (Self, Self) {
        let left = SessionMuxCore::new(
            session_id.clone(),
            config,
            TransportRouteKind::TestMemory,
            "memory:left",
            "memory:right",
        );
        let right = SessionMuxCore::new(
            session_id,
            config,
            TransportRouteKind::TestMemory,
            "memory:right",
            "memory:left",
        );
        spawn_memory_direction(Arc::clone(&left), Arc::clone(&right));
        spawn_memory_direction(Arc::clone(&right), Arc::clone(&left));
        (
            Self {
                core: Arc::clone(&left),
                peer: Arc::downgrade(&right),
            },
            Self {
                core: right,
                peer: Arc::downgrade(&left),
            },
        )
    }
}

impl Drop for MemoryTransportMux {
    fn drop(&mut self) {
        self.core.terminate_now(None);
        if let Some(peer) = self.peer.upgrade() {
            peer.terminate_now(Some("memory transport peer dropped".into()));
        }
    }
}

fn spawn_memory_direction(source: Arc<SessionMuxCore>, target: Arc<SessionMuxCore>) {
    for lane in TransportLane::ALL {
        let source = Arc::clone(&source);
        let target = Arc::clone(&target);
        let owner = Arc::clone(&source);
        let task = tokio::spawn(async move {
            while let Some(envelope) = source.next_outbound(lane).await {
                if target.deliver(envelope).await.is_err() {
                    break;
                }
            }
        });
        owner.register_task(task);
    }
}

#[async_trait::async_trait]
impl TransportMuxPort for MemoryTransportMux {
    async fn send(&self, envelope: TransportEnvelope) -> Result<TransportSendOutcome> {
        self.core.submit(envelope).await
    }

    async fn recv(&self, lane: TransportLane) -> Result<Option<TransportEnvelope>> {
        self.core.recv(lane).await
    }

    async fn route_snapshot(&self) -> TransportRouteSnapshot {
        self.core.snapshot().await
    }

    async fn close(&self) -> Result<()> {
        self.core.close().await;
        if let Some(peer) = self.peer.upgrade() {
            peer.fail("memory transport peer closed").await;
        }
        Ok(())
    }
}
