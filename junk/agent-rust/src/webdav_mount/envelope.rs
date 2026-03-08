use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountEnvelope {
    pub version: u16,
    pub mount_id: u64,
    pub request_id: u64,
    pub kind: String,
    #[serde(default)]
    pub flags: u32,
    #[serde(default)]
    pub root_path: String,
    #[serde(default)]
    pub auth: Option<MountAuth>,
    #[serde(default)]
    pub op: Option<MountOp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountAuth {
    pub url: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountOp {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub dst_path: Option<String>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub length: Option<u64>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub bytes_b64: Option<String>,
}

impl MountEnvelope {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| anyhow!("invalid mount envelope: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_envelope_with_default_flags() {
        let raw =
            br#"{"version":1,"mount_id":7,"request_id":11,"kind":"open","root_path":"/data"}"#;
        let env = MountEnvelope::from_bytes(raw).expect("should parse");
        assert_eq!(env.flags, 0);
        assert_eq!(env.kind, "open");
    }

    #[test]
    fn parse_op_envelope() {
        let raw = br#"{"version":1,"mount_id":7,"request_id":12,"kind":"op","op":{"name":"read","path":"/a.txt","offset":0,"length":10}}"#;
        let env = MountEnvelope::from_bytes(raw).expect("should parse");
        assert_eq!(env.op.expect("op").name, "read");
    }
}
