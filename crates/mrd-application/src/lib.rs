// mrd-application: Application use case layer
//
// Orchestrates session lifecycle, signaling, transport, and media
// through well-defined use cases. Depends on abstract ports rather
// than concrete implementations.

#![warn(missing_docs)]

use mrd_proto::{SessionId, DeviceId};
use mrd_signal_proto::{SignalMessage, IceCandidate};
use anyhow::Result;

/// Abstract ports for external dependencies
///
/// These traits define the boundaries between the application layer
/// and infrastructure adapters. This allows the application logic to
/// be tested independently and swapped between implementations.
pub mod ports {
    use super::*;

    /// Signaling client port - handles communication with signaling server
    #[async_trait::async_trait]
    pub trait SignalingPort: Send + Sync {
        /// Drain pending signaling events
        async fn drain_events(&self, handle: u64) -> Result<Vec<SignalMessage>>;

        /// Get device ID for a registration handle
        async fn device_id(&self, handle: u64) -> Result<DeviceId>;
    }

    /// Session coordinator port - manages session state and signaling metadata
    pub trait SessionCoordinatorPort: Send + Sync {
        /// Request a new session as controller
        fn request_session(
            &mut self,
            session_id: SessionId,
            source_device_id: DeviceId,
            target_device_id: DeviceId,
            transport: String,
            listen_addr: Option<String>,
            server_name: Option<String>,
            cert_der_b64: Option<String>,
        ) -> Result<()>;

        /// Accept an incoming session as agent
        fn accept_session(
            &mut self,
            session_id: SessionId,
            transport: String,
            listen_addr: Option<String>,
            server_name: Option<String>,
            cert_der_b64: Option<String>,
        ) -> Result<()>;

        /// Apply a remote WebRTC offer
        fn apply_remote_offer(&mut self, session_id: SessionId, sdp: String) -> Result<()>;

        /// Apply a remote WebRTC answer
        fn apply_remote_answer(&mut self, session_id: SessionId, sdp: String) -> Result<()>;

        /// Apply a remote ICE candidate
        fn apply_remote_ice_candidate(&mut self, session_id: SessionId, candidate: IceCandidate) -> Result<()>;

        /// Get a snapshot of session state
        fn snapshot(&self, session_id: &SessionId) -> Option<SessionSnapshot>;
    }

    /// Session snapshot DTO
    #[derive(Debug, Clone)]
    pub struct SessionSnapshot {
        pub session_id: SessionId,
        pub transport: String,
        pub source_device_id: Option<DeviceId>,
        pub target_device_id: Option<DeviceId>,
        pub local_listen_addr: Option<String>,
        pub local_server_name: Option<String>,
        pub local_cert_der_b64: Option<String>,
        pub remote_listen_addr: Option<String>,
        pub remote_server_name: Option<String>,
        pub remote_cert_der_b64: Option<String>,
        /// Explicit lifecycle state from domain model
        pub lifecycle_state: String,
        /// Last error if any
        pub last_error: Option<String>,
    }

    /// QUIC host port - manages QUIC transport connection
    #[async_trait::async_trait]
    pub trait QuicHostPort: Send + Sync {
        /// Sync host state from session snapshot
        async fn sync_from_session_snapshot(
            &self,
            local_device_id: &DeviceId,
            session_id: &SessionId,
            snapshot: &SessionSnapshot,
        ) -> Result<()>;
    }
}

/// Application use cases
pub mod usecases {
    use super::*;

    /// Apply signaling events to session coordinators
    ///
    /// This use case drains events from the signaling client and applies
    /// them to the appropriate session coordinators (QUIC or WebRTC).
    pub async fn apply_realtime_events(
        signaling: &dyn ports::SignalingPort,
        webrtc_sessions: &mut dyn ports::SessionCoordinatorPort,
        quic_sessions: &mut dyn ports::SessionCoordinatorPort,
        handle: u64,
    ) -> Result<Option<SessionId>> {
        let events = signaling.drain_events(handle).await?;
        let mut last_session_id: Option<SessionId> = None;

        for event in events {
            match event {
                SignalMessage::SessionRequest(request) => {
                    last_session_id = Some(request.session_id.clone());
                    if request.transport == "quic_quinn" {
                        quic_sessions.request_session(
                            request.session_id,
                            request.source_device_id,
                            request.target_device_id,
                            request.transport,
                            request.quic_listen_addr,
                            request.quic_server_name,
                            request.quic_cert_der_b64,
                        )?;
                    }
                }
                SignalMessage::SessionAccept(accept) => {
                    last_session_id = Some(accept.session_id.clone());
                    if accept.transport == "quic_quinn" {
                        quic_sessions.accept_session(
                            accept.session_id,
                            accept.transport,
                            accept.quic_listen_addr,
                            accept.quic_server_name,
                            accept.quic_cert_der_b64,
                        )?;
                    }
                }
                SignalMessage::WebrtcOffer(description) => {
                    last_session_id = Some(description.session_id.clone());
                    webrtc_sessions.apply_remote_offer(description.session_id, description.sdp)?;
                }
                SignalMessage::WebrtcAnswer(description) => {
                    last_session_id = Some(description.session_id.clone());
                    webrtc_sessions.apply_remote_answer(description.session_id, description.sdp)?;
                }
                SignalMessage::IceCandidate(candidate) => {
                    last_session_id = Some(candidate.session_id.clone());
                    webrtc_sessions.apply_remote_ice_candidate(candidate.session_id.clone(), candidate)?;
                }
                _ => {}
            }
        }

        Ok(last_session_id)
    }

    /// Sync QUIC host from session snapshot
    ///
    /// This use case synchronizes the QUIC transport host with the
    /// current session state from the session coordinator.
    pub async fn sync_quic_host_from_session_snapshot(
        quic_host: &dyn ports::QuicHostPort,
        quic_sessions: &dyn ports::SessionCoordinatorPort,
        local_device_id: &DeviceId,
        session_id: &SessionId,
    ) -> Result<()> {
        let snapshot = quic_sessions.snapshot(session_id);
        if let Some(snapshot) = snapshot {
            quic_host.sync_from_session_snapshot(local_device_id, session_id, &snapshot).await?;
        }
        Ok(())
    }

    /// Start a new controller session
    pub fn start_session() -> Result<()> {
        Ok(())
    }

    /// Accept an incoming agent session
    pub fn accept_session() -> Result<()> {
        Ok(())
    }

    /// Synchronize runtime state
    pub fn sync_runtime() -> Result<()> {
        Ok(())
    }
}

/// Re-exports
pub use ports::*;
pub use usecases::*;
