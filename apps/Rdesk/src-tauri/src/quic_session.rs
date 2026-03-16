use std::collections::HashMap;

use mrd_proto::{DeviceId, SessionId};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuicSessionSnapshot {
    pub transport: String,
    pub source_device_id: Option<String>,
    pub target_device_id: Option<String>,
    pub local_listen_addr: Option<String>,
    pub local_server_name: Option<String>,
    pub local_cert_der_b64: Option<String>,
    pub remote_listen_addr: Option<String>,
    pub remote_server_name: Option<String>,
    pub remote_cert_der_b64: Option<String>,
}

#[derive(Debug, Default)]
pub struct QuicSessionCoordinator {
    sessions: HashMap<SessionId, QuicSessionSnapshot>,
}

impl QuicSessionCoordinator {
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
        Ok(())
    }

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
        Ok(())
    }

    pub fn snapshot(&self, session_id: &SessionId) -> Option<&QuicSessionSnapshot> {
        self.sessions.get(session_id)
    }
}

#[cfg(test)]
mod tests {
    use mrd_proto::{DeviceId, SessionId};

    use super::QuicSessionCoordinator;

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
        assert_eq!(snapshot.local_listen_addr.as_deref(), Some("127.0.0.1:5000"));
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
        assert_eq!(snapshot.remote_listen_addr.as_deref(), Some("127.0.0.1:6000"));
        assert_eq!(snapshot.remote_server_name.as_deref(), Some("localhost"));
        assert_eq!(snapshot.remote_cert_der_b64.as_deref(), Some("BAUG"));
    }
}
