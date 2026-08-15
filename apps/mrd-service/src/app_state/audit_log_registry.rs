use super::{now_unix_ms, AUDIT_EVENT_LIMIT};
use mrd_ipc::{
    AuditEvent, AuditEventMetadataV2, AuditEventPageV2, AuditEventV2, AuditEventsQueryV2,
    AuditLogQuery, DecimalU64, RemoteCursorState, RemotePermissionScope, RemoteRouteKind,
};
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
    #[cfg(test)]
    failed_actions: Mutex<std::collections::HashSet<String>>,
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
            #[cfg(test)]
            failed_actions: Mutex::new(std::collections::HashSet::new()),
        }
    }
}

impl AuditLogRegistry {
    pub(crate) fn persistent(store: Arc<PersistentStore>) -> Self {
        Self {
            backend: AuditLogBackend::Persistent(store),
            #[cfg(test)]
            failed_actions: Mutex::new(std::collections::HashSet::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_action(&self, action: impl Into<String>) {
        self.failed_actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(action.into());
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
        #[cfg(test)]
        if self
            .failed_actions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&action)
        {
            return Err(StoreError::InvalidAuditEvent);
        }
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

    /// Query a bounded, typed, content-free audit projection with an exclusive cursor.
    pub fn query_v2(&self, query: &AuditEventsQueryV2) -> Result<AuditEventPageV2, StoreError> {
        validate_v2_query(query)?;
        match &self.backend {
            AuditLogBackend::InMemory(log) => {
                let log = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                Ok(build_in_memory_v2_page(&log, query))
            }
            AuditLogBackend::Persistent(store) => build_persistent_v2_page(store, query),
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

fn validate_v2_query(query: &AuditEventsQueryV2) -> Result<(), StoreError> {
    let bounded = |value: Option<&str>, max_bytes: usize| {
        value.is_none_or(|value| value.len() <= max_bytes && !value.contains('\0'))
    };
    if query.limit == 0
        || query.limit > AUDIT_EVENT_LIMIT as u32
        || !bounded(
            query.session_id.as_ref().map(|value| value.0.as_str()),
            MAX_CORRELATION_ID_BYTES,
        )
        || !bounded(query.action.as_deref(), MAX_ACTION_BYTES)
        || !bounded(query.outcome.as_deref(), MAX_OUTCOME_BYTES)
        || !bounded(
            query.peer_device_id.as_ref().map(|value| value.0.as_str()),
            MAX_CORRELATION_ID_BYTES,
        )
    {
        return Err(StoreError::InvalidAuditQuery);
    }
    Ok(())
}

fn build_in_memory_v2_page(log: &InMemoryAuditLog, query: &AuditEventsQueryV2) -> AuditEventPageV2 {
    let after_sequence = query.after_sequence.map(DecimalU64::get);
    if let (Some(after), Some(oldest)) = (after_sequence, log.events.front().map(|event| event.id))
    {
        if after.saturating_add(1) < oldest {
            return AuditEventPageV2 {
                events: Vec::new(),
                next_after_sequence: log
                    .next_id
                    .checked_sub(1)
                    .filter(|sequence| *sequence > 0)
                    .map(DecimalU64::new),
                cursor_state: RemoteCursorState::ResetRequired,
                has_more: false,
                chain_verified: false,
            };
        }
    }

    let matching =
        log.events
            .iter()
            .filter(|event| after_sequence.is_none_or(|after| event.id > after))
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
            .filter(|event| {
                query
                    .outcome
                    .as_ref()
                    .is_none_or(|outcome| event.outcome == *outcome)
            })
            .filter(|event| {
                query.peer_device_id.as_ref().is_none_or(|peer_device_id| {
                    event.peer_device_id.as_ref() == Some(peer_device_id)
                })
            })
            .collect::<Vec<_>>();
    let limit = query.limit as usize;
    let (selected, has_more) = if after_sequence.is_some() {
        (
            matching.iter().take(limit).copied().collect::<Vec<_>>(),
            matching.len() > limit,
        )
    } else {
        let start = matching.len().saturating_sub(limit);
        (matching[start..].to_vec(), false)
    };
    let events = selected
        .into_iter()
        .cloned()
        .map(project_v2_event)
        .collect::<Vec<_>>();
    let next_after_sequence = events
        .last()
        .map(|event| event.sequence)
        .or(query.after_sequence);
    AuditEventPageV2 {
        events,
        next_after_sequence,
        cursor_state: RemoteCursorState::Current,
        has_more,
        chain_verified: true,
    }
}

fn build_persistent_v2_page(
    store: &PersistentStore,
    query: &AuditEventsQueryV2,
) -> Result<AuditEventPageV2, StoreError> {
    let after_sequence = query.after_sequence.map(DecimalU64::get);
    let fetch_limit = query.limit.saturating_add(1).min(AUDIT_EVENT_LIMIT as u32);
    let records = store.query_audit(&AuditQuery {
        after_sequence,
        limit: fetch_limit,
        session_id: query.session_id.as_ref().map(|value| value.0.clone()),
        action: query.action.clone(),
        outcome: query.outcome.clone(),
        peer_device_id: query.peer_device_id.as_ref().map(|value| value.0.clone()),
    })?;
    let limit = query.limit as usize;
    let mut events = records
        .into_iter()
        .map(project_record)
        .map(project_v2_event)
        .collect::<Vec<_>>();
    let mut has_more = false;
    if after_sequence.is_some() {
        has_more = events.len() > limit;
        events.truncate(limit);
        if !has_more && query.limit == AUDIT_EVENT_LIMIT as u32 {
            if let Some(last) = events.last() {
                has_more = !store
                    .query_audit(&AuditQuery {
                        after_sequence: Some(last.sequence.get()),
                        limit: 1,
                        session_id: query.session_id.as_ref().map(|value| value.0.clone()),
                        action: query.action.clone(),
                        outcome: query.outcome.clone(),
                        peer_device_id: query.peer_device_id.as_ref().map(|value| value.0.clone()),
                    })?
                    .is_empty();
            }
        }
    } else if events.len() > limit {
        events = events.split_off(events.len() - limit);
    }
    let next_after_sequence = events
        .last()
        .map(|event| event.sequence)
        .or(query.after_sequence);
    Ok(AuditEventPageV2 {
        events,
        next_after_sequence,
        cursor_state: RemoteCursorState::Current,
        has_more,
        chain_verified: true,
    })
}

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

fn project_v2_event(event: AuditEvent) -> AuditEventV2 {
    let peer_key_id = unique_detail_value(&event.details, "peer_key_id").map(str::to_string);
    let metadata = AuditEventMetadataV2 {
        authorization_state: parse_detail(&event.details, "authorization_state"),
        access_mode: parse_detail(&event.details, "access_mode"),
        route_state: parse_detail(&event.details, "route_state"),
        media_state: parse_detail(&event.details, "media_state"),
        requested_scopes: parse_scope_detail(&event.details, "requested_scopes"),
        granted_scopes: parse_scope_detail(&event.details, "granted_scopes"),
        policy_revision: parse_decimal_detail(&event.details, "policy_revision"),
        trust_revision: parse_decimal_detail(&event.details, "trust_revision"),
    };
    AuditEventV2 {
        sequence: DecimalU64::new(event.id),
        timestamp_ms: event.timestamp_ms,
        action: event.action,
        outcome: event.outcome,
        session_id: event.session_id,
        actor_device_id: event.actor_device_id,
        peer_device_id: event.peer_device_id,
        peer_key_id,
        transport_kind: event.transport_kind.as_deref().and_then(parse_route_kind),
        reason_code: event.reason.as_deref().and_then(parse_wire_value),
        metadata,
    }
}

fn unique_detail_value<'a>(details: &'a [(String, String)], key: &str) -> Option<&'a str> {
    let mut values = details
        .iter()
        .filter(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.as_str());
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}

fn parse_detail<T>(details: &[(String, String)], key: &str) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    unique_detail_value(details, key).and_then(parse_wire_value)
}

fn parse_wire_value<T>(value: &str) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

fn parse_decimal_detail(details: &[(String, String)], key: &str) -> Option<DecimalU64> {
    let raw = unique_detail_value(details, key)?;
    let value = raw.parse::<u64>().ok()?;
    (raw == value.to_string()).then_some(DecimalU64::new(value))
}

fn parse_scope_detail(details: &[(String, String)], key: &str) -> Vec<RemotePermissionScope> {
    let Some(raw) = unique_detail_value(details, key) else {
        return Vec::new();
    };
    let parsed = if raw.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<RemotePermissionScope>>(raw).ok()
    } else {
        raw.split(',')
            .map(str::trim)
            .map(parse_wire_value)
            .collect::<Option<Vec<_>>>()
    };
    let Some(mut scopes) = parsed else {
        return Vec::new();
    };
    scopes.sort_unstable();
    scopes.dedup();
    scopes
}

fn parse_route_kind(value: &str) -> Option<RemoteRouteKind> {
    match value {
        "quic" | "lan_quic" => Some(RemoteRouteKind::LanQuic),
        "webrtc_direct" => Some(RemoteRouteKind::WebRtcDirect),
        "webrtc_relay" => Some(RemoteRouteKind::WebRtcRelay),
        _ => None,
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
