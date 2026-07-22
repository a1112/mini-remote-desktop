use mrd_ipc::CapabilitySnapshot;

/// Cached service-owned capability snapshot exposed to UI and session handlers.
#[derive(Debug)]
pub struct CapabilitySnapshotRegistry {
    snapshot: CapabilitySnapshot,
    refresh_in_progress: bool,
}

impl Default for CapabilitySnapshotRegistry {
    fn default() -> Self {
        Self {
            snapshot: crate::capabilities::local_capability_snapshot_static(),
            refresh_in_progress: false,
        }
    }
}

impl CapabilitySnapshotRegistry {
    pub fn snapshot(&self) -> CapabilitySnapshot {
        self.snapshot.clone()
    }

    pub fn replace(&mut self, snapshot: CapabilitySnapshot) {
        self.snapshot = snapshot;
        self.refresh_in_progress = false;
    }

    pub fn begin_refresh(&mut self) -> bool {
        if self.refresh_in_progress {
            return false;
        }
        self.refresh_in_progress = true;
        true
    }

    pub fn finish_refresh(&mut self, snapshot: Option<CapabilitySnapshot>) {
        if let Some(snapshot) = snapshot {
            self.snapshot = snapshot;
        }
        self.refresh_in_progress = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_without_snapshot_keeps_cached_snapshot_and_releases_refresh_gate() {
        let mut registry = CapabilitySnapshotRegistry::default();
        let snapshot = registry.snapshot();

        assert!(registry.begin_refresh());
        assert!(!registry.begin_refresh());

        registry.finish_refresh(None);

        assert_eq!(registry.snapshot(), snapshot);
        assert!(registry.begin_refresh());
    }
}
