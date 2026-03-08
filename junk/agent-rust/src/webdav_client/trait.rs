use anyhow::Result;
use std::fmt::Debug;

use crate::webdav_client::model::{WebDavEndpoint, WebDavListEntry, WebDavStat};

pub trait WebDavClient: Send + Sync + Debug {
    fn probe(&self, endpoint: &WebDavEndpoint) -> Result<()>;
    fn stat(&self, endpoint: &WebDavEndpoint, path: &str) -> Result<WebDavStat>;
    fn list(&self, endpoint: &WebDavEndpoint, path: &str) -> Result<Vec<WebDavListEntry>>;
    fn read(
        &self,
        endpoint: &WebDavEndpoint,
        path: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>>;
}

#[derive(Debug)]
pub struct NoopWebDavClient;

impl WebDavClient for NoopWebDavClient {
    fn probe(&self, endpoint: &WebDavEndpoint) -> Result<()> {
        let _ = endpoint;
        Ok(())
    }

    fn stat(&self, endpoint: &WebDavEndpoint, path: &str) -> Result<WebDavStat> {
        let _ = endpoint;
        Ok(WebDavStat {
            path: path.to_string(),
            is_dir: true,
            size: 0,
            etag: None,
        })
    }

    fn list(&self, endpoint: &WebDavEndpoint, path: &str) -> Result<Vec<WebDavListEntry>> {
        let _ = endpoint;
        Ok(vec![WebDavListEntry {
            path: path.to_string(),
            is_dir: true,
            size: 0,
            etag: None,
        }])
    }

    fn read(
        &self,
        endpoint: &WebDavEndpoint,
        _path: &str,
        _offset: u64,
        _length: u64,
    ) -> Result<Vec<u8>> {
        let _ = endpoint;
        Ok(Vec::new())
    }
}
