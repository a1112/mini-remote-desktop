use mrd_proto::DeviceId;

/// Device registry
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    local_device: Option<(DeviceId, String)>, // (id, name)
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

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::DeviceId;

    #[test]
    fn unregistered_registry_accepts_fallback_registration() {
        let mut registry = DeviceRegistry::default();

        let registered = registry
            .register_if_unregistered(
                DeviceId("fallback-device".to_string()),
                "Fallback Device".to_string(),
            )
            .expect("fallback registration");

        assert_eq!(registered.0, DeviceId("fallback-device".to_string()));
        assert_eq!(registered.1, "Fallback Device");
        assert!(registry.is_registered());
    }
}
