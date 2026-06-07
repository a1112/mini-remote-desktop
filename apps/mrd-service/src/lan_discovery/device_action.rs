use mrd_ipc::{DeviceActionKind, DeviceActionResult};
use mrd_proto::DeviceId;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const LAN_DEVICE_ACTION_TRANSPORT: &str = "device_action_control_v1";

static LAN_DEVICE_ACTION_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn next_device_action_request_id() -> u64 {
    LAN_DEVICE_ACTION_REQUEST_COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
        .max(1)
}

pub(crate) fn accept_device_action_request(
    local_device_id: Option<&str>,
    target_device_id: &str,
    action: DeviceActionKind,
) -> DeviceActionResult {
    let is_local_target = local_device_id.is_some_and(|device_id| device_id == target_device_id);
    let (accepted, supported, message) = if is_local_target {
        match action {
            DeviceActionKind::RemoteTerminal => (
                true,
                false,
                "Remote terminal request reserved; waiting for explicit peer consent and a service-owned command executor.",
            ),
            DeviceActionKind::Restart => (
                false,
                false,
                "Remote restart requires explicit peer consent and a privileged service executor.",
            ),
            DeviceActionKind::Shutdown => (
                false,
                false,
                "Remote shutdown requires explicit peer consent and a privileged service executor.",
            ),
            DeviceActionKind::Disconnect => (
                false,
                false,
                "Disconnect is handled by the requesting service session registry.",
            ),
            DeviceActionKind::WakeOnLan => (
                false,
                false,
                "Wake-on-LAN must be sent by the requesting device before the peer is awake.",
            ),
        }
    } else {
        (
            false,
            false,
            "Device action target does not match this local service.",
        )
    };

    DeviceActionResult {
        device_id: DeviceId(target_device_id.to_string()),
        action,
        accepted,
        supported,
        message: message.to_string(),
    }
}
