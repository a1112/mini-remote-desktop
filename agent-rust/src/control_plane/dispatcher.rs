use anyhow::{Result, anyhow};

use crate::control_plane::mount_protocol;
use crate::security::secret_store::SecretStore;
use crate::webdav_mount::envelope::MountEnvelope;
use crate::webdav_mount::manager::{MountManager, MountManagerResponse};

#[derive(Debug)]
pub struct MountDispatcher {
    manager: MountManager,
    _secret_store: SecretStore,
}

impl MountDispatcher {
    pub fn new() -> Self {
        Self {
            manager: MountManager::new(),
            _secret_store: SecretStore::default(),
        }
    }

    pub fn on_file_mount(
        &mut self,
        op: u8,
        mount_id: u64,
        flags: u32,
        path: String,
    ) -> Result<MountManagerResponse> {
        match op {
            mount_protocol::MOUNT_OPEN
            | mount_protocol::MOUNT_LIST
            | mount_protocol::MOUNT_CLOSE
            | mount_protocol::MOUNT_HEARTBEAT
            | mount_protocol::MOUNT_CAPS_QUERY => self.manager.handle(op, mount_id, flags, path),
            _ => Err(anyhow!("unsupported mount op={op}")),
        }
    }

    pub fn on_mount_envelope(&mut self, envelope: &MountEnvelope) -> Result<MountManagerResponse> {
        let kind = envelope.kind.to_ascii_lowercase();
        match kind.as_str() {
            "open" => self.manager.open_from_envelope(
                envelope.mount_id,
                envelope.flags,
                envelope.root_path.clone(),
                envelope.auth.as_ref(),
            ),
            "list" => self.on_file_mount(
                mount_protocol::MOUNT_LIST,
                envelope.mount_id,
                envelope.flags,
                String::new(),
            ),
            "close" => self.on_file_mount(
                mount_protocol::MOUNT_CLOSE,
                envelope.mount_id,
                envelope.flags,
                String::new(),
            ),
            "heartbeat" => self.on_file_mount(
                mount_protocol::MOUNT_HEARTBEAT,
                envelope.mount_id,
                envelope.flags,
                String::new(),
            ),
            "caps" | "caps_query" => self.on_file_mount(
                mount_protocol::MOUNT_CAPS_QUERY,
                envelope.mount_id,
                envelope.flags,
                String::new(),
            ),
            "op" => {
                let op = envelope
                    .op
                    .as_ref()
                    .ok_or_else(|| anyhow!("mount envelope kind=op missing op payload"))?;
                self.manager.execute_op(envelope.mount_id, op)
            }
            _ => Err(anyhow!("unsupported mount envelope kind={}", envelope.kind)),
        }
    }
}

impl Default for MountDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webdav_mount::envelope::{MountAuth, MountOp};

    #[test]
    fn reject_unsupported_mount_op() {
        let mut d = MountDispatcher::new();
        let err = d
            .on_file_mount(0xFF, 1, 0, "/".to_string())
            .expect_err("should fail");
        assert!(err.to_string().contains("unsupported mount op"));
    }

    #[test]
    fn envelope_open_maps_and_opens_session() {
        let mut d = MountDispatcher::new();
        let env = MountEnvelope {
            version: 1,
            mount_id: 42,
            request_id: 99,
            kind: "open".to_string(),
            flags: mount_protocol::FLAG_READ_ONLY,
            root_path: "/shared".to_string(),
            auth: Some(MountAuth {
                url: "http://127.0.0.1:19080".to_string(),
                username: None,
                password_ref: None,
            }),
            op: None,
        };
        let resp = d.on_mount_envelope(&env).expect("open should succeed");
        match resp {
            MountManagerResponse::Opened { mount_id } => assert_eq!(mount_id, 42),
            _ => panic!("unexpected response"),
        }
    }

    #[test]
    fn envelope_op_stat_returns_result() {
        let mut d = MountDispatcher::new();
        let open = MountEnvelope {
            version: 1,
            mount_id: 7,
            request_id: 1,
            kind: "open".to_string(),
            flags: 0,
            root_path: "/shared".to_string(),
            auth: Some(MountAuth {
                url: "http://127.0.0.1:19080".to_string(),
                username: None,
                password_ref: None,
            }),
            op: None,
        };
        d.on_mount_envelope(&open).expect("open");

        let env = MountEnvelope {
            version: 1,
            mount_id: 7,
            request_id: 2,
            kind: "op".to_string(),
            flags: 0,
            root_path: String::new(),
            auth: None,
            op: Some(MountOp {
                name: "stat".to_string(),
                path: "/".to_string(),
                dst_path: None,
                offset: None,
                length: None,
                etag: None,
                bytes_b64: None,
            }),
        };
        let resp = d.on_mount_envelope(&env).expect("op should succeed");
        match resp {
            MountManagerResponse::OpResult { mount_id, result } => {
                assert_eq!(mount_id, 7);
                assert!(result.contains("stat path"));
            }
            _ => panic!("unexpected response"),
        }
    }

    #[test]
    fn envelope_unknown_kind_rejected() {
        let mut d = MountDispatcher::new();
        let env = MountEnvelope {
            version: 1,
            mount_id: 1,
            request_id: 1,
            kind: "noop".to_string(),
            flags: 0,
            root_path: "/".to_string(),
            auth: None,
            op: None,
        };
        let err = d.on_mount_envelope(&env).expect_err("kind should fail");
        assert!(err.to_string().contains("unsupported mount envelope kind"));
    }
}
