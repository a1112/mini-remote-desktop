// mrd-session: Session domain model
//
// Defines session aggregates, roles, and state without depending
// on concrete infrastructure implementations.

#![warn(missing_docs)]

pub mod scheduler;

use mrd_proto::{BackendRole, DeviceId, SessionId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Session lifecycle state - explicit state machine for session progression
///
/// This enum represents the authoritative lifecycle state of a session.
/// Unlike inferred states from bootstrap metadata, this state is explicitly
/// tracked and transitioned by the service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SessionLifecycleState {
    /// Session created but not yet listening or connecting
    Created,
    /// Local transport is listening for incoming connections (agent role)
    Listening,
    /// Actively connecting to remote peer (controller role)
    Connecting,
    /// Transport connection established
    Connected,
    /// Media streaming active
    Streaming,
    /// Session failed with error message
    Failed { message: String },
    /// Session closed cleanly
    Closed,
}

impl SessionLifecycleState {
    /// Check if this is an active state (can transition to streaming)
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Listening | Self::Connecting | Self::Connected | Self::Streaming
        )
    }

    /// Check if this is a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::Closed)
    }

    /// Get the string representation for IPC serialization
    pub fn as_str(&self) -> &str {
        match self {
            Self::Created => "created",
            Self::Listening => "listening",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Streaming => "streaming",
            Self::Failed { .. } => "failed",
            Self::Closed => "closed",
        }
    }
}

impl Default for SessionLifecycleState {
    fn default() -> Self {
        Self::Created
    }
}

/// Capability set for a device
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilitySet {
    pub supports_webrtc: bool,
    pub supports_quic: bool,
}

/// Session plan containing routing and capability information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionPlan {
    pub session_id: SessionId,
    pub initiator: DeviceId,
    pub target: DeviceId,
    pub role: BackendRole,
    pub capabilities: CapabilitySet,
}

/// QUIC session snapshot (domain state, independent of Quinn)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicSessionSnapshot {
    /// Transport protocol identifier
    pub transport: String,
    /// Source device ID (controller)
    pub source_device_id: Option<String>,
    /// Target device ID (agent)
    pub target_device_id: Option<String>,
    /// Local listen address
    pub local_listen_addr: Option<String>,
    /// Local server name (SNI)
    pub local_server_name: Option<String>,
    /// Local certificate (DER, base64-encoded)
    pub local_cert_der_b64: Option<String>,
    /// Remote listen address
    pub remote_listen_addr: Option<String>,
    /// Remote server name
    pub remote_server_name: Option<String>,
    /// Remote certificate (DER, base64-encoded)
    pub remote_cert_der_b64: Option<String>,
    /// Explicit lifecycle state
    pub lifecycle_state: SessionLifecycleState,
    /// Last error message if any
    pub last_error: Option<String>,
}

impl Default for QuicSessionSnapshot {
    fn default() -> Self {
        Self {
            transport: String::new(),
            source_device_id: None,
            target_device_id: None,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::default(),
            last_error: None,
        }
    }
}

/// QUIC session coordinator - manages QUIC session state at domain level
#[derive(Debug, Default)]
pub struct QuicSessionCoordinator {
    sessions: HashMap<SessionId, QuicSessionSnapshot>,
}

impl QuicSessionCoordinator {
    /// Request a new QUIC session as controller
    pub fn request_session(
        &mut self,
        session_id: SessionId,
        source_device_id: DeviceId,
        target_device_id: DeviceId,
        transport: String,
        local_listen_addr: Option<String>,
        local_server_name: Option<String>,
        local_cert_der_b64: Option<String>,
    ) -> Result<(), String> {
        let snapshot = self.sessions.entry(session_id).or_default();
        snapshot.transport = transport;
        snapshot.source_device_id = Some(source_device_id.0);
        snapshot.target_device_id = Some(target_device_id.0);
        snapshot.local_listen_addr = local_listen_addr;
        snapshot.local_server_name = local_server_name;
        snapshot.local_cert_der_b64 = local_cert_der_b64;
        snapshot.lifecycle_state = SessionLifecycleState::Connecting;
        Ok(())
    }

    /// Accept an incoming QUIC session as agent
    pub fn accept_session(
        &mut self,
        session_id: SessionId,
        transport: String,
        remote_listen_addr: Option<String>,
        remote_server_name: Option<String>,
        remote_cert_der_b64: Option<String>,
    ) -> Result<(), String> {
        let snapshot = self.sessions.entry(session_id).or_default();
        snapshot.transport = transport;
        snapshot.remote_listen_addr = remote_listen_addr;
        snapshot.remote_server_name = remote_server_name;
        snapshot.remote_cert_der_b64 = remote_cert_der_b64;
        snapshot.lifecycle_state = SessionLifecycleState::Listening;
        Ok(())
    }

    /// Transition session to connected state
    pub fn set_connected(&mut self, session_id: &SessionId) -> Result<(), String> {
        let snapshot = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session not found: {}", session_id.0))?;
        snapshot.lifecycle_state = SessionLifecycleState::Connected;
        Ok(())
    }

    /// Transition session to streaming state
    pub fn set_streaming(&mut self, session_id: &SessionId) -> Result<(), String> {
        let snapshot = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session not found: {}", session_id.0))?;
        snapshot.lifecycle_state = SessionLifecycleState::Streaming;
        Ok(())
    }

    /// Mark session as failed
    pub fn set_failed(&mut self, session_id: &SessionId, message: String) -> Result<(), String> {
        let snapshot = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session not found: {}", session_id.0))?;
        snapshot.lifecycle_state = SessionLifecycleState::Failed {
            message: message.clone(),
        };
        snapshot.last_error = Some(message);
        Ok(())
    }

    /// Close session
    pub fn close(&mut self, session_id: &SessionId) -> Result<(), String> {
        let snapshot = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session not found: {}", session_id.0))?;
        snapshot.lifecycle_state = SessionLifecycleState::Closed;
        Ok(())
    }

    /// Get a snapshot of session state
    pub fn snapshot(&self, session_id: &SessionId) -> Option<&QuicSessionSnapshot> {
        self.sessions.get(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requesting_quic_session_records_transport_and_local_bootstrap() {
        let mut coordinator = QuicSessionCoordinator::default();

        coordinator
            .request_session(
                SessionId("session-quic".into()),
                DeviceId("controller-1".into()),
                DeviceId("agent-1".into()),
                "quic_quinn".into(),
                Some("127.0.0.1:5000".into()),
                Some("localhost".into()),
                Some("AQID".into()),
            )
            .expect("request quic session");

        let snapshot = coordinator
            .snapshot(&SessionId("session-quic".into()))
            .expect("quic request snapshot");

        assert_eq!(snapshot.transport, "quic_quinn");
        assert_eq!(snapshot.source_device_id.as_deref(), Some("controller-1"));
        assert_eq!(snapshot.target_device_id.as_deref(), Some("agent-1"));
        assert_eq!(
            snapshot.local_listen_addr.as_deref(),
            Some("127.0.0.1:5000")
        );
        assert_eq!(snapshot.local_server_name.as_deref(), Some("localhost"));
        assert_eq!(snapshot.local_cert_der_b64.as_deref(), Some("AQID"));
        assert_eq!(snapshot.remote_listen_addr, None);
    }

    #[test]
    fn accepting_quic_session_records_remote_bootstrap() {
        let mut coordinator = QuicSessionCoordinator::default();

        coordinator
            .accept_session(
                SessionId("session-quic".into()),
                "quic_quinn".into(),
                Some("127.0.0.1:6000".into()),
                Some("localhost".into()),
                Some("BAUG".into()),
            )
            .expect("accept quic session");

        let snapshot = coordinator
            .snapshot(&SessionId("session-quic".into()))
            .expect("quic accept snapshot");

        assert_eq!(snapshot.transport, "quic_quinn");
        assert_eq!(
            snapshot.remote_listen_addr.as_deref(),
            Some("127.0.0.1:6000")
        );
        assert_eq!(snapshot.remote_server_name.as_deref(), Some("localhost"));
        assert_eq!(snapshot.remote_cert_der_b64.as_deref(), Some("BAUG"));
    }
}
