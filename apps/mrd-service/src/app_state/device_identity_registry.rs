use super::now_unix_ms;
use mrd_ipc::PairedDeviceIdentity;
use mrd_proto::DeviceId;
use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::DeviceId;

    #[test]
    fn upsert_preserves_certificate_fingerprint_and_revoke_updates_trust() {
        let device_id = DeviceId("peer-device".to_string());
        let mut registry = DeviceIdentityRegistry::default();

        registry.upsert(
            device_id.clone(),
            Some("sha256:first".to_string()),
            "trusted",
        );
        registry.upsert(device_id.clone(), None, "paired");

        let paired = registry.list();
        assert_eq!(paired.len(), 1);
        assert_eq!(
            paired[0].certificate_fingerprint.as_deref(),
            Some("sha256:first")
        );
        assert_eq!(paired[0].trust_status, "paired");

        registry.revoke(&device_id);

        let revoked = registry.list();
        assert_eq!(revoked[0].trust_status, "revoked");
        assert_eq!(
            revoked[0].certificate_fingerprint.as_deref(),
            Some("sha256:first")
        );
    }
}
