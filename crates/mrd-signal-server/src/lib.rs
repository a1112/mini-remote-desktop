use mrd_proto::{DeviceId, SessionId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRoute {
    pub controller: DeviceId,
    pub agent: DeviceId,
}

#[derive(Debug, Default)]
pub struct SessionRouter {
    routes: HashMap<SessionId, SessionRoute>,
}

impl SessionRouter {
    pub fn register(&mut self, session_id: SessionId, route: SessionRoute) {
        self.routes.insert(session_id, route);
    }

    pub fn resolve_peer(
        &self,
        session_id: &SessionId,
        sender: &DeviceId,
    ) -> Result<DeviceId, SessionRouteError> {
        let route = self
            .routes
            .get(session_id)
            .ok_or(SessionRouteError::UnknownSession)?;

        if &route.controller == sender {
            return Ok(route.agent.clone());
        }

        if &route.agent == sender {
            return Ok(route.controller.clone());
        }

        Err(SessionRouteError::UnknownSender)
    }
}

#[derive(Debug, Error)]
pub enum SessionRouteError {
    #[error("unknown session")]
    UnknownSession,
    #[error("unknown sender")]
    UnknownSender,
}

#[cfg(test)]
mod tests {
    use super::{SessionRoute, SessionRouter};
    use mrd_proto::{DeviceId, SessionId};

    #[test]
    fn resolves_peer_from_session_route() {
        let mut router = SessionRouter::default();
        let session_id = SessionId("session-1".into());
        let controller = DeviceId("controller-1".into());
        let agent = DeviceId("agent-1".into());

        router.register(
            session_id.clone(),
            SessionRoute {
                controller: controller.clone(),
                agent: agent.clone(),
            },
        );

        let peer = router
            .resolve_peer(&session_id, &controller)
            .expect("resolve controller peer");

        assert_eq!(peer, agent);
    }
}
