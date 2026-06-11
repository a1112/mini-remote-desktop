use mrd_ipc::{DevicePreference, DevicePreferenceUpdate};
use mrd_proto::DeviceId;
use std::collections::HashMap;

/// Service-owned device preference flags.
#[derive(Debug, Default)]
pub struct DevicePreferenceRegistry {
    preferences: HashMap<DeviceId, DevicePreference>,
}

impl DevicePreferenceRegistry {
    pub fn list(&self) -> Vec<DevicePreference> {
        let mut preferences = self.preferences.values().cloned().collect::<Vec<_>>();
        preferences.sort_by(|a, b| a.device_id.0.cmp(&b.device_id.0));
        preferences
    }

    pub fn update(
        &mut self,
        device_id: DeviceId,
        update: DevicePreferenceUpdate,
    ) -> DevicePreference {
        let preference = self
            .preferences
            .entry(device_id.clone())
            .or_insert_with(|| DevicePreference {
                device_id,
                favorite: false,
                disabled: false,
                removed: false,
            });
        if let Some(favorite) = update.favorite {
            preference.favorite = favorite;
        }
        if let Some(disabled) = update.disabled {
            preference.disabled = disabled;
        }
        if let Some(removed) = update.removed {
            preference.removed = removed;
        }
        preference.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_merges_partial_device_preference_flags() {
        let device_id = DeviceId("device-a".to_string());
        let mut registry = DevicePreferenceRegistry::default();

        registry.update(
            device_id.clone(),
            DevicePreferenceUpdate {
                favorite: Some(true),
                disabled: None,
                removed: None,
            },
        );
        let preference = registry.update(
            device_id,
            DevicePreferenceUpdate {
                favorite: None,
                disabled: Some(true),
                removed: None,
            },
        );

        assert!(preference.favorite);
        assert!(preference.disabled);
        assert!(!preference.removed);
    }
}
