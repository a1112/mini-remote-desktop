use super::SessionRegistry;
use mrd_ipc::{CaptureSourceSelection, DisplayMode, DisplayModeChange, MediaProfileNegotiation};
use mrd_proto::SessionId;
use std::collections::HashMap;

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
