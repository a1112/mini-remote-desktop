use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::process::{Child, Command, Stdio};

#[derive(Debug, Clone)]
pub struct RcloneMountSpec {
    pub mount_id: u64,
    pub remote: String,
    pub mountpoint: String,
    pub network_mode: bool,
    pub volume_name: Option<String>,
}

#[derive(Debug, Default)]
pub struct RcloneMountManager {
    children: HashMap<u64, Child>,
}

impl RcloneMountManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mount(&mut self, spec: &RcloneMountSpec) -> Result<()> {
        if self.children.contains_key(&spec.mount_id) {
            return Err(anyhow!(
                "rclone mount already exists mount_id={}",
                spec.mount_id
            ));
        }
        let binary = std::env::var("AGENT_RCLONE_BIN").unwrap_or_else(|_| "rclone".to_string());
        let mut cmd = Command::new(binary);
        cmd.arg("mount")
            .arg(&spec.remote)
            .arg(&spec.mountpoint)
            .arg("--vfs-cache-mode")
            .arg("writes")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if spec.network_mode {
            cmd.arg("--network-mode");
        }
        if let Some(v) = spec.volume_name.as_ref() {
            cmd.arg("--volname").arg(v);
        }
        let child = cmd.spawn()?;
        self.children.insert(spec.mount_id, child);
        Ok(())
    }

    pub fn unmount(&mut self, mount_id: u64) -> Result<()> {
        let mut child = self
            .children
            .remove(&mount_id)
            .ok_or_else(|| anyhow!("rclone mount not found mount_id={mount_id}"))?;
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

impl Drop for RcloneMountManager {
    fn drop(&mut self) {
        for (_id, child) in self.children.iter_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmount_missing_fails() {
        let mut m = RcloneMountManager::new();
        let err = m.unmount(10).expect_err("should fail");
        assert!(err.to_string().contains("not found"));
    }
}
