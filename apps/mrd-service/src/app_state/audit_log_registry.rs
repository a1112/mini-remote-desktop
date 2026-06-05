use super::{now_unix_ms, AUDIT_EVENT_LIMIT};
use mrd_ipc::{AuditEvent, AuditLogQuery};
use mrd_proto::{DeviceId, SessionId};
use std::collections::VecDeque;

/// In-memory service audit event registry.
#[derive(Debug)]
pub struct AuditLogRegistry {
    next_id: u64,
    events: VecDeque<AuditEvent>,
    max_events: usize,
}

impl Default for AuditLogRegistry {
    fn default() -> Self {
        Self {
            next_id: 1,
            events: VecDeque::new(),
            max_events: AUDIT_EVENT_LIMIT,
        }
    }
}

impl AuditLogRegistry {
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        action: impl Into<String>,
        outcome: impl Into<String>,
        session_id: Option<SessionId>,
        actor_device_id: Option<DeviceId>,
        peer_device_id: Option<DeviceId>,
        transport_kind: Option<String>,
        reason: Option<String>,
        details: Vec<(String, String)>,
    ) -> AuditEvent {
        let event = AuditEvent {
            id: self.next_id,
            timestamp_ms: now_unix_ms(),
            action: action.into(),
            outcome: outcome.into(),
            session_id,
            actor_device_id,
            peer_device_id,
            transport_kind,
            reason,
            details,
        };
        self.next_id = self.next_id.saturating_add(1);
        self.events.push_back(event.clone());
        while self.events.len() > self.max_events {
            self.events.pop_front();
        }
        event
    }

    pub fn query(&self, query: &AuditLogQuery) -> Vec<AuditEvent> {
        let mut events = self
            .events
            .iter()
            .filter(|event| {
                query
                    .session_id
                    .as_ref()
                    .is_none_or(|session_id| event.session_id.as_ref() == Some(session_id))
            })
            .filter(|event| {
                query
                    .action
                    .as_ref()
                    .is_none_or(|action| event.action == *action)
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(limit) = query.limit {
            let limit = limit as usize;
            if events.len() > limit {
                events = events.split_off(events.len() - limit);
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_ipc::AuditLogQuery;
    use mrd_proto::SessionId;

    #[test]
    fn query_limit_returns_latest_matching_events() {
        let session_id = SessionId("session-a".to_string());
        let mut registry = AuditLogRegistry::default();

        registry.record(
            "control_input",
            "accepted",
            Some(session_id.clone()),
            None,
            None,
            None,
            None,
            vec![("sequence".to_string(), "1".to_string())],
        );
        registry.record(
            "control_input",
            "accepted",
            Some(session_id.clone()),
            None,
            None,
            None,
            None,
            vec![("sequence".to_string(), "2".to_string())],
        );
        registry.record(
            "session",
            "accepted",
            Some(session_id.clone()),
            None,
            None,
            None,
            None,
            vec![("sequence".to_string(), "3".to_string())],
        );

        let events = registry.query(&AuditLogQuery {
            session_id: Some(session_id),
            action: Some("control_input".to_string()),
            limit: Some(1),
        });

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "control_input");
        assert_eq!(events[0].details[0].1, "2");
    }
}
