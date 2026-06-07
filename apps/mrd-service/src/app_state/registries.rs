use super::SessionRegistry;
use mrd_ipc::{
    AuditEvent, AuditLogQuery, CaptureSourceSelection, DisplayMode, DisplayModeChange,
    MediaProfileNegotiation, PairedDeviceIdentity,
};
use mrd_proto::{DeviceId, SessionId};
use std::collections::{HashMap, VecDeque};

const AUDIT_EVENT_LIMIT: usize = 1_000;

/// Runtime media profile negotiation state keyed by session.
#[derive(Debug, Default)]
pub struct MediaProfileRegistry {
    profiles: HashMap<SessionId, MediaProfileNegotiation>,
}

impl MediaProfileRegistry {
    pub fn set(&mut self, session_id: SessionId, negotiation: MediaProfileNegotiation) {
        self.profiles.insert(session_id, negotiation);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<MediaProfileNegotiation> {
        self.profiles.get(session_id).cloned()
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<MediaProfileNegotiation> {
        self.profiles.remove(session_id)
    }
}

/// Runtime capture source selection state keyed by session.
#[derive(Debug, Default)]
pub struct CaptureSourceRegistry {
    selections: HashMap<SessionId, CaptureSourceSelection>,
}

impl CaptureSourceRegistry {
    pub fn set(&mut self, session_id: SessionId, selection: CaptureSourceSelection) {
        self.selections.insert(session_id, selection);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<CaptureSourceSelection> {
        self.selections.get(session_id).cloned()
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<CaptureSourceSelection> {
        self.selections.remove(session_id)
    }

    pub fn active_window_capture_count(&self, sessions: &SessionRegistry) -> usize {
        self.selections
            .iter()
            .filter(|(session_id, selection)| {
                selection.source.source_kind == "window"
                    && sessions.get(session_id).is_some_and(|snapshot| {
                        snapshot.sender_active && !snapshot.lifecycle_state.is_terminal()
                    })
            })
            .count()
    }
}

/// Runtime display mode changes keyed by session.
#[derive(Debug, Default)]
pub struct DisplayModeRegistry {
    modes: HashMap<SessionId, DisplayModeState>,
}

#[derive(Debug, Clone)]
struct DisplayModeState {
    original: Option<DisplayMode>,
    active: Option<DisplayMode>,
    restore_required: bool,
}

impl DisplayModeRegistry {
    pub fn record_change(
        &mut self,
        session_id: SessionId,
        requested: DisplayMode,
        previous: Option<DisplayMode>,
        active: DisplayMode,
        restore_required: bool,
    ) -> DisplayModeChange {
        let original = previous.clone().or_else(|| {
            self.modes
                .get(&session_id)
                .and_then(|state| state.original.clone())
        });
        self.modes.insert(
            session_id.clone(),
            DisplayModeState {
                original: original.clone(),
                active: Some(active.clone()),
                restore_required,
            },
        );
        DisplayModeChange {
            session_id,
            requested: Some(requested),
            previous,
            active: Some(active),
            status: "changed".to_string(),
            reason: None,
            restore_required,
        }
    }

    pub fn record_restore(
        &mut self,
        session_id: SessionId,
        previous: DisplayMode,
        active: DisplayMode,
    ) -> DisplayModeChange {
        self.modes.remove(&session_id);
        DisplayModeChange {
            session_id,
            requested: None,
            previous: Some(previous),
            active: Some(active),
            status: "restored".to_string(),
            reason: None,
            restore_required: false,
        }
    }

    pub fn restore_mode(&self, session_id: &SessionId) -> Option<DisplayMode> {
        self.modes
            .get(session_id)
            .filter(|state| state.restore_required)
            .and_then(|state| state.original.clone())
    }

    pub fn active_mode(&self, session_id: &SessionId) -> Option<DisplayMode> {
        self.modes
            .get(session_id)
            .and_then(|state| state.active.clone())
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<DisplayMode> {
        self.modes
            .remove(session_id)
            .and_then(|state| state.original)
    }
}

/// Peer media capabilities observed for each active session.
#[derive(Debug, Default)]
pub struct SessionPeerMediaCapabilityRegistry {
    capabilities: HashMap<SessionId, Vec<String>>,
}

impl SessionPeerMediaCapabilityRegistry {
    pub fn set(&mut self, session_id: SessionId, capabilities: Vec<String>) {
        self.capabilities.insert(session_id, capabilities);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<Vec<String>> {
        self.capabilities.get(session_id).cloned()
    }

    pub fn supports(&self, session_id: &SessionId, capability: &str) -> bool {
        self.capabilities
            .get(session_id)
            .map(|capabilities| capabilities.iter().any(|value| value == capability))
            .unwrap_or(false)
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<Vec<String>> {
        self.capabilities.remove(session_id)
    }
}

/// Device registry.
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    local_device: Option<(DeviceId, String)>,
}

impl DeviceRegistry {
    pub fn register(&mut self, device_id: DeviceId, device_name: String) {
        self.local_device = Some((device_id, device_name));
    }

    pub fn register_if_unregistered(
        &mut self,
        device_id: DeviceId,
        device_name: String,
    ) -> Option<(DeviceId, String)> {
        if self.local_device.is_none() {
            self.register(device_id, device_name);
        }
        self.local_device.clone()
    }

    pub fn get_local_device(&self) -> Option<&(DeviceId, String)> {
        self.local_device.as_ref()
    }

    pub fn is_registered(&self) -> bool {
        self.local_device.is_some()
    }
}

/// In-memory paired device identity registry.
#[derive(Debug, Default)]
pub struct DeviceIdentityRegistry {
    paired_devices: HashMap<DeviceId, PairedDeviceIdentity>,
}

impl DeviceIdentityRegistry {
    pub fn upsert(
        &mut self,
        device_id: DeviceId,
        certificate_fingerprint: Option<String>,
        trust_status: impl Into<String>,
    ) {
        let display_name = device_id.0.clone();
        let existing = self.paired_devices.remove(&device_id);
        let certificate_fingerprint = certificate_fingerprint.or_else(|| {
            existing
                .as_ref()
                .and_then(|identity| identity.certificate_fingerprint.clone())
        });
        self.paired_devices.insert(
            device_id.clone(),
            PairedDeviceIdentity {
                display_name: existing
                    .as_ref()
                    .map(|identity| identity.display_name.clone())
                    .unwrap_or(display_name),
                device_id,
                certificate_fingerprint,
                trust_status: trust_status.into(),
                last_seen_ms: Some(now_unix_ms()),
            },
        );
    }

    pub fn revoke(&mut self, device_id: &DeviceId) {
        if let Some(identity) = self.paired_devices.get_mut(device_id) {
            identity.trust_status = "revoked".to_string();
            identity.last_seen_ms = Some(now_unix_ms());
        } else {
            self.upsert(device_id.clone(), None, "revoked");
        }
    }

    pub fn list(&self) -> Vec<PairedDeviceIdentity> {
        let mut identities = self.paired_devices.values().cloned().collect::<Vec<_>>();
        identities.sort_by(|a, b| a.device_id.0.cmp(&b.device_id.0));
        identities
    }
}

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

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
