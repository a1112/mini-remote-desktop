use anyhow::{Result, anyhow};

use crate::file_ops::policy::FilePolicy;
use crate::webdav_client::model::{WebDavEndpoint, WebDavListEntry, WebDavStat};
use crate::webdav_client::r#trait::WebDavClient;

#[derive(Debug, Clone)]
pub enum FileOpRequest {
    Stat {
        path: String,
    },
    List {
        path: String,
    },
    Read {
        path: String,
        offset: u64,
        length: u64,
    },
    Write {
        path: String,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub enum FileOpResponse {
    Stat(WebDavStat),
    List(Vec<WebDavListEntry>),
    Read(Vec<u8>),
    WriteAck,
}

#[derive(Debug, Clone)]
pub struct FileOpService {
    policy: FilePolicy,
}

impl FileOpService {
    pub fn new(policy: FilePolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> FilePolicy {
        self.policy
    }

    pub fn handle(
        &self,
        client: &dyn WebDavClient,
        endpoint: &WebDavEndpoint,
        request: FileOpRequest,
    ) -> Result<FileOpResponse> {
        match request {
            FileOpRequest::Stat { path } => Ok(FileOpResponse::Stat(client.stat(endpoint, &path)?)),
            FileOpRequest::List { path } => Ok(FileOpResponse::List(client.list(endpoint, &path)?)),
            FileOpRequest::Read {
                path,
                offset,
                length,
            } => Ok(FileOpResponse::Read(
                client.read(endpoint, &path, offset, length)?,
            )),
            FileOpRequest::Write { .. } => {
                if self.policy.read_only {
                    return Err(anyhow!("write is forbidden by read_only policy"));
                }
                Err(anyhow!("write op not implemented yet"))
            }
        }
    }
}

impl Default for FileOpService {
    fn default() -> Self {
        Self::new(FilePolicy::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webdav_client::model::WebDavEndpoint;
    use crate::webdav_client::r#trait::NoopWebDavClient;

    #[test]
    fn read_only_policy_rejects_write() {
        let service = FileOpService::new(FilePolicy {
            read_only: true,
            allow_delete: false,
            allow_move: false,
            allow_overwrite: false,
        });
        let endpoint = WebDavEndpoint {
            base_url: "noop://".to_string(),
            root_path: "/".to_string(),
            username: None,
            password_ref: None,
        };
        let err = service
            .handle(
                &NoopWebDavClient,
                &endpoint,
                FileOpRequest::Write {
                    path: "/a.txt".to_string(),
                    bytes: vec![1],
                },
            )
            .expect_err("write should fail");
        assert!(err.to_string().contains("read_only"));
    }
}
