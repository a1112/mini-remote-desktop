use super::now_unix_ms;
use mrd_identity::DeviceIdentity;
use mrd_ipc::PairedDeviceIdentity;
use mrd_proto::DeviceId;
use mrd_store_sqlite::{
    AuditDraft, AuditRecord, AuditedTrustTransition, PersistentStore, StoreError, TrustRecord,
    TrustState,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Pairing/trust adapter. Production trust is pinned by authenticated Ed25519 key ID.
pub struct DeviceIdentityRegistry {
    backend: DeviceIdentityBackend,
}

enum DeviceIdentityBackend {
    InMemory(Mutex<HashMap<DeviceId, PairedDeviceIdentity>>),
    Persistent {
        store: Arc<PersistentStore>,
        machine_identity: Arc<DeviceIdentity>,
    },
}

#[derive(Debug)]
pub enum DeviceIdentityRegistryError {
    AuthenticatedPeerRequired,
    Store(StoreError),
}

impl std::fmt::Display for DeviceIdentityRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthenticatedPeerRequired => {
                formatter.write_str("an authenticated peer public key is required")
            }
            Self::Store(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DeviceIdentityRegistryError {}

impl From<StoreError> for DeviceIdentityRegistryError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl std::fmt::Debug for DeviceIdentityRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceIdentityRegistry")
            .field(
                "backend",
                &match self.backend {
                    DeviceIdentityBackend::InMemory(_) => "in_memory_test_fake",
                    DeviceIdentityBackend::Persistent { .. } => "persistent",
                },
            )
            .finish()
    }
}

impl Default for DeviceIdentityRegistry {
    fn default() -> Self {
        Self {
            backend: DeviceIdentityBackend::InMemory(Mutex::new(HashMap::new())),
        }
    }
}

impl DeviceIdentityRegistry {
    pub(crate) fn persistent(
        store: Arc<PersistentStore>,
        machine_identity: DeviceIdentity,
    ) -> Self {
        Self {
            backend: DeviceIdentityBackend::Persistent {
                store,
                machine_identity: Arc::new(machine_identity),
            },
        }
    }

    pub fn machine_key_id(&self) -> Option<&str> {
        match &self.backend {
            DeviceIdentityBackend::InMemory(_) => None,
            DeviceIdentityBackend::Persistent {
                machine_identity, ..
            } => Some(machine_identity.key_id()),
        }
    }

    pub fn machine_public_key(&self) -> Option<&[u8]> {
        match &self.backend {
            DeviceIdentityBackend::InMemory(_) => None,
            DeviceIdentityBackend::Persistent {
                machine_identity, ..
            } => Some(machine_identity.public_key()),
        }
    }

    pub fn upsert(
        &self,
        device_id: DeviceId,
        certificate_fingerprint: Option<String>,
        trust_status: impl Into<String>,
    ) -> Result<(), DeviceIdentityRegistryError> {
        let DeviceIdentityBackend::InMemory(paired_devices) = &self.backend else {
            return Err(DeviceIdentityRegistryError::AuthenticatedPeerRequired);
        };
        let mut paired_devices = paired_devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let display_name = device_id.0.clone();
        let existing = paired_devices.remove(&device_id);
        let certificate_fingerprint = certificate_fingerprint.or_else(|| {
            existing
                .as_ref()
                .and_then(|identity| identity.certificate_fingerprint.clone())
        });
        paired_devices.insert(
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
        Ok(())
    }

    pub fn revoke(&self, device_id: &DeviceId) -> Result<(), DeviceIdentityRegistryError> {
        let DeviceIdentityBackend::InMemory(paired_devices) = &self.backend else {
            return Err(DeviceIdentityRegistryError::AuthenticatedPeerRequired);
        };
        let mut paired_devices = paired_devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(identity) = paired_devices.get_mut(device_id) {
            identity.trust_status = "revoked".to_string();
            identity.last_seen_ms = Some(now_unix_ms());
        } else {
            drop(paired_devices);
            self.upsert(device_id.clone(), None, "revoked")?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<PairedDeviceIdentity>, DeviceIdentityRegistryError> {
        match &self.backend {
            DeviceIdentityBackend::InMemory(paired_devices) => {
                let paired_devices = paired_devices
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut identities = paired_devices.values().cloned().collect::<Vec<_>>();
                identities.sort_by(|a, b| a.device_id.0.cmp(&b.device_id.0));
                Ok(identities)
            }
            DeviceIdentityBackend::Persistent { .. } => Ok(Vec::new()),
        }
    }

    pub fn approve_authenticated_peer(
        &self,
        peer_key_id: &str,
        public_key: &[u8],
        epoch: u64,
        audit: AuditDraft,
    ) -> Result<(TrustRecord, AuditRecord), DeviceIdentityRegistryError> {
        let DeviceIdentityBackend::Persistent { store, .. } = &self.backend else {
            return Err(DeviceIdentityRegistryError::AuthenticatedPeerRequired);
        };
        store
            .insert_trusted_device_with_audit(
                peer_key_id,
                public_key,
                epoch,
                TrustState::Trusted,
                audit,
            )
            .map_err(Into::into)
    }

    pub fn transition_authenticated_peer(
        &self,
        peer_key_id: &str,
        expected_revision: u64,
        next: TrustState,
        audit: AuditDraft,
    ) -> Result<AuditedTrustTransition, DeviceIdentityRegistryError> {
        let DeviceIdentityBackend::Persistent { store, .. } = &self.backend else {
            return Err(DeviceIdentityRegistryError::AuthenticatedPeerRequired);
        };
        store
            .transition_trust_with_audit(peer_key_id, expected_revision, next, audit)
            .map_err(Into::into)
    }

    pub fn trusted_records(
        &self,
        include_revoked: bool,
    ) -> Result<Vec<TrustRecord>, DeviceIdentityRegistryError> {
        let DeviceIdentityBackend::Persistent { store, .. } = &self.backend else {
            return Err(DeviceIdentityRegistryError::AuthenticatedPeerRequired);
        };
        store
            .list_trusted_devices(include_revoked)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_preserves_certificate_fingerprint_and_revoke_updates_trust() {
        let device_id = DeviceId("peer-device".to_string());
        let registry = DeviceIdentityRegistry::default();

        registry
            .upsert(
                device_id.clone(),
                Some("sha256:first".to_string()),
                "trusted",
            )
            .unwrap();
        registry.upsert(device_id.clone(), None, "paired").unwrap();

        let paired = registry.list().unwrap();
        assert_eq!(paired.len(), 1);
        assert_eq!(
            paired[0].certificate_fingerprint.as_deref(),
            Some("sha256:first")
        );
        assert_eq!(paired[0].trust_status, "paired");

        registry.revoke(&device_id).unwrap();

        let revoked = registry.list().unwrap();
        assert_eq!(revoked[0].trust_status, "revoked");
        assert_eq!(
            revoked[0].certificate_fingerprint.as_deref(),
            Some("sha256:first")
        );
    }
}
