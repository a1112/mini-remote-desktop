use crate::{session_authorization::VerifiedIncomingAuthorizationRequest, AppState};
use anyhow::{bail, Context, Result};
use mrd_application::{
    AuthenticatedSessionSignal, AuthenticatedSessionSignalPort, SessionLifecycleState,
    SessionSnapshot, VerifiedSignalingEvent,
};
use mrd_ipc::{RemoteAccessMode, RemotePermissionScope};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
struct WebRtcGrantBinding {
    peer_key_id: String,
    accepted_fingerprints: Vec<String>,
}

/// Applies verified WAN signaling to the service's authoritative aggregates.
pub struct ServiceSignalingMapper {
    app_state: Arc<AppState>,
    webrtc_grants: Mutex<HashMap<mrd_proto::SessionId, WebRtcGrantBinding>>,
}

impl ServiceSignalingMapper {
    /// Bind the mapper to the service-owned application state.
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self {
            app_state,
            webrtc_grants: Mutex::new(HashMap::new()),
        }
    }

    async fn apply_intent(
        &self,
        event: &VerifiedSignalingEvent,
        session_id: mrd_proto::SessionId,
        idempotency_key: [u8; 16],
        requested_transport: String,
    ) -> Result<()> {
        let _authorization_guard = self.app_state.authorization_security_gate.lock().await;
        if let Some(existing) = self
            .app_state
            .sessions
            .lock()
            .await
            .get(&session_id)
            .cloned()
        {
            if existing.source_device_id.as_ref() == Some(&event.sender.device_id)
                && existing.transport == requested_transport
            {
                let authorization = self
                    .app_state
                    .session_authorizations
                    .snapshot_at(&session_id, event.sender.issued_at_ms)
                    .await
                    .context("existing session has no authenticated authorization aggregate")?;
                if authorization.peer_device_id == event.sender.device_id
                    && authorization.peer_key_id == event.sender.key_id
                    && authorization.role == mrd_ipc::RemoteSessionRole::Agent
                {
                    return Ok(());
                }
            }
            bail!("signaling session identifier is already bound to another peer");
        }

        let interactive_scopes = vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard,
        ];
        self.app_state
            .session_authorizations
            .begin_verified_incoming(VerifiedIncomingAuthorizationRequest {
                session_id: session_id.clone(),
                peer_device_id: event.sender.device_id.clone(),
                peer_key_id: event.sender.key_id.clone(),
                peer_key_epoch: 1,
                access_mode: RemoteAccessMode::Attended,
                requested_scopes: interactive_scopes.clone(),
                peer_permission_ceiling: interactive_scopes.clone(),
                machine_permission_ceiling: interactive_scopes.clone(),
                runtime_capabilities: interactive_scopes,
                transport_kind: requested_transport.clone(),
                request_nonce: idempotency_key,
                created_at_ms: event.sender.issued_at_ms,
                expires_at_ms: event.sender.expires_at_ms,
            })
            .await
            .map_err(|failure| anyhow::anyhow!(failure.message))?;
        if let Err(failure) = self
            .app_state
            .session_authorizations
            .bind_authenticated_peer_key(
                &session_id,
                &event.sender.public_key,
                event.sender.issued_at_ms,
            )
            .await
        {
            let _ = self
                .app_state
                .session_authorizations
                .record_failure(
                    &session_id,
                    mrd_ipc::RemoteAuthorizationState::Denied,
                    failure.clone(),
                    event.sender.issued_at_ms,
                )
                .await;
            bail!(failure.message);
        }
        self.app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id,
                transport: requested_transport,
                source_device_id: Some(event.sender.device_id.clone()),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: SessionLifecycleState::Created,
                last_error: None,
                sender_active: false,
                receiver_active: false,
            },
        );
        Ok(())
    }

    async fn update_session<F>(
        &self,
        event: &VerifiedSignalingEvent,
        session_id: &mrd_proto::SessionId,
        update: F,
    ) -> Result<()>
    where
        F: FnOnce(&mut SessionSnapshot) -> Result<()>,
    {
        let authorization = self
            .app_state
            .session_authorizations
            .snapshot_at(session_id, event.sender.issued_at_ms)
            .await;
        let mut sessions = self.app_state.sessions.lock().await;
        let mut snapshot = sessions
            .get(session_id)
            .cloned()
            .with_context(|| format!("signaling session not found: {}", session_id.0))?;
        let expected_peer = snapshot
            .target_device_id
            .as_ref()
            .or(snapshot.source_device_id.as_ref());
        if expected_peer != Some(&event.sender.device_id) {
            bail!("signaling sender is not the session peer");
        }
        if let Some(authorization) = authorization {
            if authorization.peer_device_id != event.sender.device_id
                || authorization.peer_key_id != event.sender.key_id
            {
                bail!("signaling sender key does not match the session authorization");
            }
        }
        update(&mut snapshot)?;
        sessions.insert(session_id.clone(), snapshot);
        Ok(())
    }

    async fn require_webrtc_fingerprint(
        &self,
        event: &VerifiedSignalingEvent,
        session_id: &mrd_proto::SessionId,
        fingerprint: &str,
    ) -> Result<()> {
        let grants = self.webrtc_grants.lock().await;
        let binding = grants
            .get(session_id)
            .context("WebRTC signaling arrived without an authenticated grant")?;
        if binding.peer_key_id != event.sender.key_id
            || !binding
                .accepted_fingerprints
                .iter()
                .any(|accepted| accepted == fingerprint)
        {
            bail!("WebRTC candidate is not bound to the authenticated grant");
        }
        Ok(())
    }

    async fn require_webrtc_grant(
        &self,
        event: &VerifiedSignalingEvent,
        session_id: &mrd_proto::SessionId,
    ) -> Result<()> {
        let grants = self.webrtc_grants.lock().await;
        let binding = grants
            .get(session_id)
            .context("WebRTC signaling arrived without an authenticated grant")?;
        if binding.peer_key_id != event.sender.key_id {
            bail!("WebRTC signaling peer does not match the authenticated grant");
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl AuthenticatedSessionSignalPort for ServiceSignalingMapper {
    async fn apply_authenticated_signal(&self, event: VerifiedSignalingEvent) -> Result<()> {
        match event.signal.clone() {
            AuthenticatedSessionSignal::AuthorizationRequested {
                session_id,
                idempotency_key,
                requested_transport,
            } => {
                self.apply_intent(&event, session_id, idempotency_key, requested_transport)
                    .await
            }
            AuthenticatedSessionSignal::Granted {
                session_id,
                accepted_transport,
                accepted_candidate_fingerprints,
            } => {
                self.update_session(&event, &session_id, |snapshot| {
                    if snapshot.lifecycle_state.is_terminal() {
                        bail!("terminal session cannot accept a signaling grant");
                    }
                    snapshot.transport = accepted_transport.clone();
                    snapshot.lifecycle_state = SessionLifecycleState::Connecting;
                    snapshot.last_error = None;
                    Ok(())
                })
                .await?;
                self.webrtc_grants.lock().await.insert(
                    session_id,
                    WebRtcGrantBinding {
                        peer_key_id: event.sender.key_id,
                        accepted_fingerprints: accepted_candidate_fingerprints,
                    },
                );
                Ok(())
            }
            AuthenticatedSessionSignal::Denied { session_id, reason } => {
                self.update_session(&event, &session_id, |snapshot| {
                    let message = format!("remote session denied: {reason:?}");
                    snapshot.lifecycle_state = SessionLifecycleState::Failed {
                        message: message.clone(),
                    };
                    snapshot.last_error = Some(message);
                    snapshot.sender_active = false;
                    snapshot.receiver_active = false;
                    Ok(())
                })
                .await?;
                self.webrtc_grants.lock().await.remove(&session_id);
                Ok(())
            }
            AuthenticatedSessionSignal::Closed { session_id, .. } => {
                self.update_session(&event, &session_id, |snapshot| {
                    snapshot.lifecycle_state = SessionLifecycleState::Closed;
                    snapshot.sender_active = false;
                    snapshot.receiver_active = false;
                    Ok(())
                })
                .await?;
                self.webrtc_grants.lock().await.remove(&session_id);
                Ok(())
            }
            AuthenticatedSessionSignal::WebRtcCandidate {
                session_id,
                candidate_fingerprint,
                ..
            } => {
                self.require_webrtc_fingerprint(&event, &session_id, &candidate_fingerprint)
                    .await
            }
            AuthenticatedSessionSignal::WebRtcOffer {
                session_id,
                candidate_fingerprints,
                ..
            }
            | AuthenticatedSessionSignal::WebRtcAnswer {
                session_id,
                candidate_fingerprints,
                ..
            } => {
                self.require_webrtc_grant(&event, &session_id).await?;
                for fingerprint in candidate_fingerprints {
                    self.require_webrtc_fingerprint(&event, &session_id, &fingerprint)
                        .await?;
                }
                Ok(())
            }
        }
    }
}
