use anyhow::{Result, anyhow};
use base64::Engine;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::control_plane::mount_protocol;
use crate::file_ops::policy::FilePolicy;
use crate::file_ops::service::{FileOpRequest, FileOpResponse, FileOpService};
use crate::rclone_mount::manager::{RcloneMountManager, RcloneMountSpec};
use crate::webdav_client::model::WebDavEndpoint;
use crate::webdav_client::reqwest_impl::ReqwestWebDavClient;
use crate::webdav_client::r#trait::{NoopWebDavClient, WebDavClient};
use crate::webdav_mount::envelope::{MountAuth, MountOp};
use crate::webdav_mount::session::MountSession;

#[derive(Debug, Clone)]
pub enum MountManagerResponse {
    Opened { mount_id: u64 },
    Listed { mount_ids: Vec<u64> },
    Closed { mount_id: u64 },
    HeartbeatOk { mount_id: u64 },
    Caps { flags_supported: u32 },
    OpResult { mount_id: u64, result: String },
}

#[derive(Debug)]
pub struct MountManager {
    sessions: HashMap<u64, MountSession>,
    client: Box<dyn WebDavClient>,
    heartbeat_timeout: Duration,
    rclone: RcloneMountManager,
}

impl MountManager {
    pub fn new() -> Self {
        let client_mode = std::env::var("AGENT_WEBDAV_CLIENT")
            .unwrap_or_else(|_| "noop".to_string())
            .to_ascii_lowercase();
        let client: Box<dyn WebDavClient> = match client_mode.as_str() {
            "reqwest" => match ReqwestWebDavClient::new() {
                Ok(c) => Box::new(c),
                Err(_) => Box::new(NoopWebDavClient),
            },
            _ => Box::new(NoopWebDavClient),
        };
        Self {
            sessions: HashMap::new(),
            client,
            heartbeat_timeout: Duration::from_secs(15),
            rclone: RcloneMountManager::new(),
        }
    }

    pub fn handle(
        &mut self,
        op: u8,
        mount_id: u64,
        flags: u32,
        path: String,
    ) -> Result<MountManagerResponse> {
        self.reconcile_heartbeat(Instant::now());
        match op {
            mount_protocol::MOUNT_OPEN => self.open_with_auth(mount_id, flags, path, None),
            mount_protocol::MOUNT_LIST => Ok(self.list()),
            mount_protocol::MOUNT_CLOSE => self.close(mount_id),
            mount_protocol::MOUNT_HEARTBEAT => self.heartbeat(mount_id),
            mount_protocol::MOUNT_CAPS_QUERY => Ok(MountManagerResponse::Caps {
                flags_supported: mount_protocol::FLAG_READ_ONLY
                    | mount_protocol::FLAG_AUTO_CREATE_ROOT
                    | mount_protocol::FLAG_ALLOW_DELETE
                    | mount_protocol::FLAG_ALLOW_MOVE
                    | mount_protocol::FLAG_ALLOW_OVERWRITE
                    | mount_protocol::FLAG_STRICT_ETAG,
            }),
            _ => Err(anyhow!("unsupported mount op={op}")),
        }
    }

    pub fn open_from_envelope(
        &mut self,
        mount_id: u64,
        flags: u32,
        root_path: String,
        auth: Option<&MountAuth>,
    ) -> Result<MountManagerResponse> {
        self.open_with_auth(mount_id, flags, root_path, auth)
    }

    pub fn execute_op(&self, mount_id: u64, op: &MountOp) -> Result<MountManagerResponse> {
        let session = self
            .sessions
            .get(&mount_id)
            .ok_or_else(|| anyhow!("mount not found id={mount_id}"))?;
        let name = op.name.to_ascii_lowercase();
        let req = match name.as_str() {
            "stat" => FileOpRequest::Stat {
                path: op.path.clone(),
            },
            "list" => FileOpRequest::List {
                path: op.path.clone(),
            },
            "read" => FileOpRequest::Read {
                path: op.path.clone(),
                offset: op.offset.unwrap_or(0),
                length: op.length.unwrap_or(1024 * 1024),
            },
            "write" => FileOpRequest::Write {
                path: op.path.clone(),
                bytes: op
                    .bytes_b64
                    .as_ref()
                    .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
                    .unwrap_or_default(),
            },
            _ => return Err(anyhow!("unsupported file op={}", op.name)),
        };
        let resp = session
            .file_service
            .handle(self.client.as_ref(), &session.endpoint, req)?;
        Ok(MountManagerResponse::OpResult {
            mount_id,
            result: encode_op_response(resp),
        })
    }

    fn open_with_auth(
        &mut self,
        mount_id: u64,
        flags: u32,
        path: String,
        auth: Option<&MountAuth>,
    ) -> Result<MountManagerResponse> {
        if path.trim().is_empty() {
            return Err(anyhow!("mount path is empty"));
        }
        if self.sessions.contains_key(&mount_id) {
            return Err(anyhow!("mount already exists id={mount_id}"));
        }
        let endpoint = endpoint_from_auth_or_env(normalize_path(path), auth);
        self.client.probe(&endpoint)?;

        if std::env::var("AGENT_RCLONE_ENABLE").ok().as_deref() == Some("1") {
            let remote =
                std::env::var("AGENT_RCLONE_REMOTE").unwrap_or_else(|_| "mydav:/".to_string());
            let mountpoint =
                std::env::var("AGENT_RCLONE_MOUNTPOINT").unwrap_or_else(|_| "X:".to_string());
            let network_mode =
                std::env::var("AGENT_RCLONE_NETWORK_MODE").ok().as_deref() == Some("1");
            let volume_name = std::env::var("AGENT_RCLONE_VOLNAME").ok();
            let _ = self.rclone.mount(&RcloneMountSpec {
                mount_id,
                remote,
                mountpoint,
                network_mode,
                volume_name,
            });
        }

        let file_service = FileOpService::new(FilePolicy::from_mount_flags(flags));
        let mut session = MountSession::new(mount_id, endpoint, flags, file_service);
        session.open();
        self.sessions.insert(mount_id, session);
        Ok(MountManagerResponse::Opened { mount_id })
    }

    fn list(&self) -> MountManagerResponse {
        let mut mount_ids: Vec<u64> = self.sessions.keys().copied().collect();
        mount_ids.sort_unstable();
        MountManagerResponse::Listed { mount_ids }
    }

    fn close(&mut self, mount_id: u64) -> Result<MountManagerResponse> {
        let mut session = self
            .sessions
            .remove(&mount_id)
            .ok_or_else(|| anyhow!("mount not found id={mount_id}"))?;
        session.close();
        if std::env::var("AGENT_RCLONE_ENABLE").ok().as_deref() == Some("1") {
            let _ = self.rclone.unmount(mount_id);
        }
        Ok(MountManagerResponse::Closed { mount_id })
    }

    fn heartbeat(&mut self, mount_id: u64) -> Result<MountManagerResponse> {
        let session = self
            .sessions
            .get_mut(&mount_id)
            .ok_or_else(|| anyhow!("mount not found id={mount_id}"))?;
        session.heartbeat();
        Ok(MountManagerResponse::HeartbeatOk { mount_id })
    }

    fn reconcile_heartbeat(&mut self, now: Instant) {
        for session in self.sessions.values_mut() {
            session.apply_timeout(now, self.heartbeat_timeout);
        }
    }
}

fn endpoint_from_auth_or_env(root_path: String, auth: Option<&MountAuth>) -> WebDavEndpoint {
    if let Some(auth) = auth {
        return WebDavEndpoint {
            base_url: auth.url.clone(),
            root_path,
            username: auth.username.clone(),
            password_ref: auth.password_ref.clone(),
        };
    }
    WebDavEndpoint {
        base_url: std::env::var("AGENT_WEBDAV_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:19080".to_string()),
        root_path,
        username: std::env::var("AGENT_WEBDAV_USER").ok(),
        password_ref: std::env::var("AGENT_WEBDAV_PASS").ok(),
    }
}

fn encode_op_response(resp: FileOpResponse) -> String {
    match resp {
        FileOpResponse::Stat(v) => format!(
            "stat path={} is_dir={} size={} etag={}",
            v.path,
            v.is_dir,
            v.size,
            v.etag.unwrap_or_default()
        ),
        FileOpResponse::List(v) => format!("list entries={}", v.len()),
        FileOpResponse::Read(v) => format!("read bytes={}", v.len()),
        FileOpResponse::WriteAck => "write ok".to_string(),
    }
}

fn normalize_path(path: String) -> String {
    let p = path.trim().replace('\\', "/");
    if p.starts_with('/') {
        p
    } else {
        format!("/{p}")
    }
}

impl Default for MountManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_lifecycle_open_list_heartbeat_close() {
        let mut m = MountManager::new();
        let opened = m
            .handle(mount_protocol::MOUNT_OPEN, 10, 0, "/team/docs".to_string())
            .expect("open");
        match opened {
            MountManagerResponse::Opened { mount_id } => assert_eq!(mount_id, 10),
            _ => panic!("unexpected response"),
        }

        let listed = m
            .handle(mount_protocol::MOUNT_LIST, 0, 0, String::new())
            .expect("list");
        match listed {
            MountManagerResponse::Listed { mount_ids } => assert_eq!(mount_ids, vec![10]),
            _ => panic!("unexpected response"),
        }

        let hb = m
            .handle(mount_protocol::MOUNT_HEARTBEAT, 10, 0, String::new())
            .expect("heartbeat");
        match hb {
            MountManagerResponse::HeartbeatOk { mount_id } => assert_eq!(mount_id, 10),
            _ => panic!("unexpected response"),
        }

        let op = MountOp {
            name: "stat".to_string(),
            path: "/".to_string(),
            dst_path: None,
            offset: None,
            length: None,
            etag: None,
            bytes_b64: None,
        };
        let op_resp = m.execute_op(10, &op).expect("op stat");
        match op_resp {
            MountManagerResponse::OpResult { mount_id, .. } => assert_eq!(mount_id, 10),
            _ => panic!("unexpected response"),
        }

        let closed = m
            .handle(mount_protocol::MOUNT_CLOSE, 10, 0, String::new())
            .expect("close");
        match closed {
            MountManagerResponse::Closed { mount_id } => assert_eq!(mount_id, 10),
            _ => panic!("unexpected response"),
        }
    }
}
