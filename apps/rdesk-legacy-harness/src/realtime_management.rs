use serde::{Deserialize, Serialize};

const DEFAULT_MANAGEMENT_API_BASE: &str = "http://127.0.0.1:9530/api/v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RealtimeStatus {
    pub running: bool,
    pub reachable: bool,
    pub status: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RealtimeManagementClient {
    base_url: String,
    client: reqwest::Client,
}

impl RealtimeManagementClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("build realtime management client"),
        }
    }

    pub fn from_env() -> Self {
        let base_url = std::env::var("RDESK_MANAGEMENT_API_BASE")
            .unwrap_or_else(|_| DEFAULT_MANAGEMENT_API_BASE.to_string());
        Self::new(base_url)
    }

    pub async fn status(&self) -> Result<RealtimeStatus, String> {
        self.request_status(reqwest::Method::GET, "status").await
    }

    pub async fn start(&self) -> Result<RealtimeStatus, String> {
        self.request_status(reqwest::Method::POST, "start").await
    }

    pub async fn stop(&self) -> Result<RealtimeStatus, String> {
        self.request_status(reqwest::Method::POST, "stop").await
    }

    pub async fn restart(&self) -> Result<RealtimeStatus, String> {
        self.request_status(reqwest::Method::POST, "restart").await
    }

    async fn request_status(
        &self,
        method: reqwest::Method,
        action: &str,
    ) -> Result<RealtimeStatus, String> {
        let url = format!("{}/realtime/{}", self.base_url, action);
        let request = self.client.request(method, &url);
        let response = request
            .send()
            .await
            .map_err(|e| format!("请求 realtime 管理接口失败: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("realtime 管理接口失败 ({}): {}", status, body));
        }

        response
            .json::<RealtimeStatus>()
            .await
            .map_err(|e| format!("解析 realtime 响应失败: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::{RealtimeManagementClient, RealtimeStatus};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_mock_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock management server");
        let addr = listener.local_addr().expect("mock server addr");

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };

                tokio::spawn(async move {
                    let mut buffer = [0_u8; 2048];
                    let size = socket.read(&mut buffer).await.expect("read request");
                    let request = String::from_utf8_lossy(&buffer[..size]);
                    let line = request.lines().next().unwrap_or_default();

                    let body = if line.contains("/realtime/status")
                        || line.contains("/realtime/start")
                        || line.contains("/realtime/stop")
                        || line.contains("/realtime/restart")
                    {
                        serde_json::to_string(&RealtimeStatus {
                            running: !line.contains("/realtime/stop"),
                            reachable: true,
                            status: "ok".into(),
                            pid: Some(9532),
                        })
                        .expect("serialize status")
                    } else {
                        "{\"error\":\"not found\"}".to_string()
                    };

                    let status_line = if body.contains("\"error\"") {
                        "HTTP/1.1 404 Not Found"
                    } else {
                        "HTTP/1.1 200 OK"
                    };

                    let response = format!(
                        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    socket
                        .write_all(response.as_bytes())
                        .await
                        .expect("write response");
                });
            }
        });

        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn status_reads_management_endpoint() {
        let base = spawn_mock_server().await;
        let client = RealtimeManagementClient::new(format!("{}/api/v1", base));

        let status = client.status().await.expect("read status");

        assert!(status.reachable);
        assert_eq!(status.status, "ok");
    }

    #[tokio::test]
    async fn restart_posts_to_management_endpoint() {
        let base = spawn_mock_server().await;
        let client = RealtimeManagementClient::new(format!("{}/api/v1", base));

        let status = client.restart().await.expect("restart sidecar");

        assert!(status.running);
        assert_eq!(status.pid, Some(9532));
    }
}
