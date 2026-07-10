use super::{now_unix_ms, AUDIT_EVENT_LIMIT};
use mrd_ipc::{AuditEvent, AuditLogQuery};
use mrd_proto::{DeviceId, SessionId};
use mrd_store_sqlite::{AuditDraft, AuditQuery, AuditRecord, PersistentStore, StoreError};
use ring::digest;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

/// Service audit adapter. Production uses the sealed SQLite store; tests use a bounded fake.
pub struct AuditLogRegistry {
    backend: AuditLogBackend,
}

enum AuditLogBackend {
    InMemory(Mutex<InMemoryAuditLog>),
    Persistent(Arc<PersistentStore>),
}

struct InMemoryAuditLog {
    next_id: u64,
    events: VecDeque<AuditEvent>,
    max_events: usize,
}

impl std::fmt::Debug for AuditLogRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuditLogRegistry")
            .field(
                "backend",
                &match self.backend {
                    AuditLogBackend::InMemory(_) => "in_memory_test_fake",
                    AuditLogBackend::Persistent(_) => "persistent",
                },
            )
            .finish()
    }
}

impl Default for AuditLogRegistry {
    fn default() -> Self {
        Self {
            backend: AuditLogBackend::InMemory(Mutex::new(InMemoryAuditLog {
                next_id: 1,
                events: VecDeque::new(),
                max_events: AUDIT_EVENT_LIMIT,
            })),
        }
    }
}

impl AuditLogRegistry {
    pub(crate) fn persistent(store: Arc<PersistentStore>) -> Self {
        Self {
            backend: AuditLogBackend::Persistent(store),
        }
    }

    /// Verifies the durable store before an audited mutation begins.
    pub fn verify_integrity(&self) -> Result<(), StoreError> {
        match &self.backend {
            AuditLogBackend::InMemory(_) => Ok(()),
            AuditLogBackend::Persistent(store) => store.verify_audit_chain(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        action: impl Into<String>,
        outcome: impl Into<String>,
        session_id: Option<SessionId>,
        actor_device_id: Option<DeviceId>,
        peer_device_id: Option<DeviceId>,
        transport_kind: Option<String>,
        reason: Option<String>,
        details: Vec<(String, String)>,
    ) -> Result<AuditEvent, StoreError> {
        let action = action.into();
        let outcome = outcome.into();
        match &self.backend {
            AuditLogBackend::InMemory(log) => {
                let mut log = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let event = AuditEvent {
                    id: log.next_id,
                    timestamp_ms: now_unix_ms(),
                    action,
                    outcome,
                    session_id,
                    actor_device_id,
                    peer_device_id,
                    transport_kind,
                    reason,
                    details,
                };
                log.next_id = log.next_id.saturating_add(1);
                log.events.push_back(event.clone());
                while log.events.len() > log.max_events {
                    log.events.pop_front();
                }
                Ok(event)
            }
            AuditLogBackend::Persistent(store) => {
                let session_id = session_id.map(|value| redact_audit_correlation_id(value.0));
                let actor_device_id =
                    actor_device_id.map(|value| redact_audit_correlation_id(value.0));
                let peer_device_id =
                    peer_device_id.map(|value| redact_audit_correlation_id(value.0));
                let transport_kind = transport_kind
                    .map(|value| bounded_audit_value(value, MAX_TRANSPORT_KIND_BYTES));
                let reason = reason.map(|value| bounded_audit_value(value, MAX_REASON_CODE_BYTES));
                let details = details
                    .into_iter()
                    .take(MAX_DETAIL_ENTRIES)
                    .map(|(key, value)| {
                        (
                            bounded_detail_key(key),
                            bounded_audit_value(value, MAX_SERVICE_DETAIL_VALUE_BYTES),
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                let record = store.append_audit(AuditDraft {
                    timestamp_ms: now_unix_ms(),
                    action,
                    outcome,
                    session_id,
                    actor_device_id,
                    peer_device_id,
                    transport_kind,
                    reason_code: reason,
                    details,
                })?;
                Ok(project_record(record))
            }
        }
    }

    pub fn query(&self, query: &AuditLogQuery) -> Result<Vec<AuditEvent>, StoreError> {
        match &self.backend {
            AuditLogBackend::InMemory(log) => {
                let log = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut events =
                    log.events
                        .iter()
                        .filter(|event| {
                            query.session_id.as_ref().is_none_or(|session_id| {
                                event.session_id.as_ref() == Some(session_id)
                            })
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
                    let limit = limit.min(AUDIT_EVENT_LIMIT as u32) as usize;
                    if events.len() > limit {
                        events = events.split_off(events.len() - limit);
                    }
                }
                Ok(events)
            }
            AuditLogBackend::Persistent(store) => store
                .query_audit(&AuditQuery {
                    after_sequence: None,
                    limit: query.limit.unwrap_or(AUDIT_EVENT_LIMIT as u32),
                    session_id: query.session_id.as_ref().map(|value| value.0.clone()),
                    action: query.action.clone(),
                    outcome: None,
                    peer_device_id: None,
                })
                .map(|records| records.into_iter().map(project_record).collect()),
        }
    }
}

const MAX_ACTION_BYTES: usize = 128;
const MAX_OUTCOME_BYTES: usize = 32;
const MAX_CORRELATION_ID_BYTES: usize = 256;
const MAX_TRANSPORT_KIND_BYTES: usize = 64;
const MAX_REASON_CODE_BYTES: usize = 128;
const MAX_DETAIL_KEY_BYTES: usize = 64;
const MAX_DETAIL_ENTRIES: usize = 32;
const MAX_SERVICE_DETAIL_VALUE_BYTES: usize = 256;

fn bounded_audit_value(value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes && !value.contains('\0') {
        return value;
    }
    let digest = sha256_hex(value.as_bytes());
    let digest_bytes = max_bytes.saturating_sub("sha256:".len()).min(digest.len());
    format!("sha256:{}", &digest[..digest_bytes])
}

pub(crate) fn redact_audit_correlation_id(value: String) -> String {
    bounded_audit_value(value, MAX_CORRELATION_ID_BYTES)
}

fn bounded_detail_key(key: String) -> String {
    const FORBIDDEN: [&str; 12] = [
        "password",
        "secret",
        "private_key",
        "credential",
        "verifier",
        "proof",
        "token",
        "clipboard",
        "keystroke",
        "file_content",
        "media_payload",
        "content",
    ];
    let normalized = key.to_ascii_lowercase();
    if !key.trim().is_empty()
        && key.len() <= MAX_DETAIL_KEY_BYTES
        && !key.contains('\0')
        && !FORBIDDEN
            .iter()
            .any(|forbidden| normalized.contains(forbidden))
    {
        return key;
    }
    format!("sha256:{}", &sha256_hex(key.as_bytes())[..32])
}

fn sha256_hex(value: &[u8]) -> String {
    digest::digest(&digest::SHA256, value)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn project_record(record: AuditRecord) -> AuditEvent {
    AuditEvent {
        id: record.sequence,
        timestamp_ms: record.draft.timestamp_ms,
        action: bounded_audit_value(record.draft.action, MAX_ACTION_BYTES),
        outcome: bounded_audit_value(record.draft.outcome, MAX_OUTCOME_BYTES),
        session_id: record
            .draft
            .session_id
            .map(|value| SessionId(bounded_audit_value(value, MAX_CORRELATION_ID_BYTES))),
        actor_device_id: record
            .draft
            .actor_device_id
            .map(|value| DeviceId(bounded_audit_value(value, MAX_CORRELATION_ID_BYTES))),
        peer_device_id: record
            .draft
            .peer_device_id
            .map(|value| DeviceId(bounded_audit_value(value, MAX_CORRELATION_ID_BYTES))),
        transport_kind: record
            .draft
            .transport_kind
            .map(|value| bounded_audit_value(value, MAX_TRANSPORT_KIND_BYTES)),
        reason: record
            .draft
            .reason_code
            .map(|value| bounded_audit_value(value, MAX_REASON_CODE_BYTES)),
        details: record
            .draft
            .details
            .into_iter()
            .take(MAX_DETAIL_ENTRIES)
            .map(|(key, value)| {
                (
                    bounded_detail_key(key),
                    bounded_audit_value(value, MAX_SERVICE_DETAIL_VALUE_BYTES),
                )
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_limit_returns_latest_matching_events() {
        let session_id = SessionId("session-a".to_string());
        let registry = AuditLogRegistry::default();

        registry
            .record(
                "control_input",
                "accepted",
                Some(session_id.clone()),
                None,
                None,
                None,
                None,
                vec![("sequence".to_string(), "1".to_string())],
            )
            .unwrap();
        registry
            .record(
                "control_input",
                "accepted",
                Some(session_id.clone()),
                None,
                None,
                None,
                None,
                vec![("sequence".to_string(), "2".to_string())],
            )
            .unwrap();
        registry
            .record(
                "session",
                "accepted",
                Some(session_id.clone()),
                None,
                None,
                None,
                None,
                vec![("sequence".to_string(), "3".to_string())],
            )
            .unwrap();

        let events = registry
            .query(&AuditLogQuery {
                session_id: Some(session_id),
                action: Some("control_input".to_string()),
                limit: Some(1),
            })
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "control_input");
        assert_eq!(events[0].details[0].1, "2");
    }
}
