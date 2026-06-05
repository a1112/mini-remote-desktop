use mrd_ipc::{DisplayMode, DisplayModeChange};
use mrd_proto::SessionId;
use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_ipc::DisplayMode;
    use mrd_proto::SessionId;

    fn display_mode(id: &str, width: u32, height: u32, current: bool) -> DisplayMode {
        DisplayMode {
            id: id.to_string(),
            source_id: Some("windows:display:0".to_string()),
            width,
            height,
            refresh_hz: 144,
            bit_depth: Some(32),
            is_current: current,
        }
    }

    #[test]
    fn remove_returns_original_mode_and_clears_active_mode() {
        let session_id = SessionId("display-mode-cleanup-session".to_string());
        let original = display_mode("windows:display:0:2560x1440@144", 2560, 1440, true);
        let requested = display_mode("windows:display:0:1920x1080@144", 1920, 1080, false);
        let mut registry = DisplayModeRegistry::default();

        registry.record_change(
            session_id.clone(),
            requested.clone(),
            Some(original.clone()),
            requested.clone(),
            true,
        );

        assert_eq!(registry.active_mode(&session_id), Some(requested));
        assert_eq!(registry.remove(&session_id), Some(original));
        assert!(registry.active_mode(&session_id).is_none());
        assert!(registry.restore_mode(&session_id).is_none());
    }
}
