use anyhow::{Result, anyhow};
use reqwest::Method;
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, RANGE};

use crate::webdav_client::model::{WebDavEndpoint, WebDavListEntry, WebDavStat};
use crate::webdav_client::r#trait::WebDavClient;

#[derive(Debug)]
pub struct ReqwestWebDavClient {
    client: Client,
}

impl ReqwestWebDavClient {
    pub fn new() -> Result<Self> {
        let client = Client::builder().build()?;
        Ok(Self { client })
    }

    fn apply_auth(
        &self,
        endpoint: &WebDavEndpoint,
        req: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match (&endpoint.username, &endpoint.password_ref) {
            (Some(user), Some(pass)) => req.basic_auth(user, Some(pass)),
            _ => req,
        }
    }

    fn endpoint_url(endpoint: &WebDavEndpoint, path: &str) -> String {
        let base = endpoint.base_url.trim_end_matches('/');
        let root = endpoint.root_path.trim_matches('/');
        let rel = path.trim_matches('/');
        match (root.is_empty(), rel.is_empty()) {
            (true, true) => format!("{base}/"),
            (true, false) => format!("{base}/{rel}"),
            (false, true) => format!("{base}/{root}"),
            (false, false) => format!("{base}/{root}/{rel}"),
        }
    }

    fn parse_stat_from_response(path: &str, response: &Response) -> WebDavStat {
        let size = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        WebDavStat {
            path: path.to_string(),
            is_dir: content_type.contains("directory"),
            size,
            etag,
        }
    }

    fn parse_list_xml(xml: &str) -> Vec<WebDavListEntry> {
        let mut out = Vec::new();
        let doc = match roxmltree::Document::parse(xml) {
            Ok(v) => v,
            Err(_) => return out,
        };
        for resp in doc.descendants().filter(|n| n.has_tag_name("response")) {
            let href = resp
                .descendants()
                .find(|n| n.has_tag_name("href"))
                .and_then(|n| n.text())
                .unwrap_or("")
                .to_string();
            if href.is_empty() {
                continue;
            }
            let is_dir = resp.descendants().any(|n| n.has_tag_name("collection"));
            let size = resp
                .descendants()
                .find(|n| n.has_tag_name("getcontentlength"))
                .and_then(|n| n.text())
                .and_then(|t| t.parse::<u64>().ok())
                .unwrap_or(0);
            let etag = resp
                .descendants()
                .find(|n| n.has_tag_name("getetag"))
                .and_then(|n| n.text())
                .map(|s| s.to_string());
            out.push(WebDavListEntry {
                path: href,
                is_dir,
                size,
                etag,
            });
        }
        out
    }
}

impl WebDavClient for ReqwestWebDavClient {
    fn probe(&self, endpoint: &WebDavEndpoint) -> Result<()> {
        let url = Self::endpoint_url(endpoint, "");
        let req = self.client.request(Method::OPTIONS, &url);
        let resp = self.apply_auth(endpoint, req).send()?;
        if !resp.status().is_success() {
            return Err(anyhow!("webdav probe failed status={}", resp.status()));
        }
        Ok(())
    }

    fn stat(&self, endpoint: &WebDavEndpoint, path: &str) -> Result<WebDavStat> {
        let url = Self::endpoint_url(endpoint, path);
        let req = self.client.request(Method::HEAD, &url);
        let resp = self.apply_auth(endpoint, req).send()?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "webdav stat failed status={} path={path}",
                resp.status()
            ));
        }
        Ok(Self::parse_stat_from_response(path, &resp))
    }

    fn list(&self, endpoint: &WebDavEndpoint, path: &str) -> Result<Vec<WebDavListEntry>> {
        let url = Self::endpoint_url(endpoint, path);
        let mut headers = HeaderMap::new();
        headers.insert("Depth", HeaderValue::from_static("1"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));
        let method = Method::from_bytes(b"PROPFIND")?;
        let body = r#"<?xml version='1.0' encoding='utf-8' ?>
<d:propfind xmlns:d='DAV:'>
  <d:prop>
    <d:getcontentlength/>
    <d:getetag/>
    <d:resourcetype/>
  </d:prop>
</d:propfind>"#;
        let req = self
            .client
            .request(method, &url)
            .headers(headers)
            .body(body);
        let resp = self.apply_auth(endpoint, req).send()?;
        if !(resp.status().is_success() || resp.status().as_u16() == 207) {
            return Err(anyhow!(
                "webdav list failed status={} path={path}",
                resp.status()
            ));
        }
        let text = resp.text()?;
        Ok(Self::parse_list_xml(&text))
    }

    fn read(
        &self,
        endpoint: &WebDavEndpoint,
        path: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>> {
        let url = Self::endpoint_url(endpoint, path);
        let end = offset.saturating_add(length).saturating_sub(1);
        let mut req = self.client.request(Method::GET, &url);
        if length > 0 {
            req = req.header(RANGE, format!("bytes={offset}-{end}"));
        }
        if let Some(pass) = endpoint.password_ref.as_ref() {
            if endpoint.username.is_none() && pass.starts_with("Bearer ") {
                req = req.header(AUTHORIZATION, pass.clone());
            }
        }
        let resp = self.apply_auth(endpoint, req).send()?;
        if !(resp.status().is_success() || resp.status().as_u16() == 206) {
            return Err(anyhow!(
                "webdav read failed status={} path={path}",
                resp.status()
            ));
        }
        Ok(resp.bytes()?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_url_joining() {
        let ep = WebDavEndpoint {
            base_url: "https://dav.example.com/remote.php/dav/files/dev".to_string(),
            root_path: "/team/docs".to_string(),
            username: None,
            password_ref: None,
        };
        assert_eq!(
            ReqwestWebDavClient::endpoint_url(&ep, "a.txt"),
            "https://dav.example.com/remote.php/dav/files/dev/team/docs/a.txt"
        );
    }

    #[test]
    fn parse_propfind_list() {
        let xml = r#"<?xml version='1.0' encoding='utf-8'?>
<d:multistatus xmlns:d='DAV:'>
  <d:response>
    <d:href>/team/docs/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/team/docs/a.txt</d:href>
    <d:propstat><d:prop><d:getcontentlength>12</d:getcontentlength><d:getetag>\"x\"</d:getetag></d:prop></d:propstat>
  </d:response>
</d:multistatus>"#;
        let items = ReqwestWebDavClient::parse_list_xml(xml);
        assert_eq!(items.len(), 2);
        assert!(items[0].is_dir);
        assert_eq!(items[1].size, 12);
    }
}
