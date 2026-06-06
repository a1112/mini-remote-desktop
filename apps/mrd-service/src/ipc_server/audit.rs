use super::IpcServer;
use mrd_ipc::IpcResponse;
use mrd_proto::{DeviceId, SessionId};

impl IpcServer {
    pub(super) async fn local_device_id(&self) -> Option<DeviceId> {
        self.app_state
            .devices
            .lock()
            .await
            .get_local_device()
            .map(|(device_id, _)| device_id.clone())
    }

    pub(super) async fn session_audit_context(
        &self,
        session_id: &SessionId,
    ) -> (Option<DeviceId>, Option<String>) {
        let sessions = self.app_state.sessions.lock().await;
        let Some(snapshot) = sessions.get(session_id) else {
            return (None, None);
        };
        let peer_device_id = snapshot
            .target_device_id
            .clone()
            .or_else(|| snapshot.source_device_id.clone());
        (peer_device_id, Some(snapshot.transport.clone()))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn record_audit_event(
        &self,
        action: impl Into<String>,
        outcome: impl Into<String>,
        session_id: Option<SessionId>,
        actor_device_id: Option<DeviceId>,
        peer_device_id: Option<DeviceId>,
        transport_kind: Option<String>,
        reason: Option<String>,
        details: Vec<(String, String)>,
    ) {
        self.app_state.audit_log.lock().await.record(
            action,
            outcome,
            session_id,
            actor_device_id,
            peer_device_id,
            transport_kind,
            reason,
            details,
        );
    }
}

pub(super) fn audit_outcome(response: &IpcResponse) -> (&'static str, Option<String>) {
    match response {
        IpcResponse::Error { message, .. } => ("error", Some(message.clone())),
        _ => ("success", None),
    }
}
