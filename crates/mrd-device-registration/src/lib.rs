//! Authenticated device enrollment and refresh against the backend API.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};
pub mod machine_identity;
pub use machine_identity::stable_machine_identity;

pub const DEVICE_CREDENTIAL_REJECTED: &str = "设备凭据已失效，请向管理员申请更新设备凭据";

#[derive(Serialize)]
pub struct DeviceRegistrationRequest {
    pub motherboard_serial: String,
    pub hostname: String,
    pub os_version: String,
    pub device_name: Option<String>,
    pub cpu_info: Option<String>,
    pub total_memory_mb: Option<u64>,
    pub gpu_info: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct DeviceRegistrationResponse {
    pub device_id: String,
    pub device_name: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

impl Drop for DeviceRegistrationResponse {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(token) = &mut self.refresh_token {
            token.zeroize();
        }
    }
}

impl std::fmt::Debug for DeviceRegistrationResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceRegistrationResponse")
            .field("device_id", &self.device_id)
            .field("device_name", &self.device_name)
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

pub async fn register_device(
    api_base: &str,
    payload: &DeviceRegistrationRequest,
    enrollment_token: Option<&str>,
    device_token: Option<&str>,
) -> Result<DeviceRegistrationResponse, &'static str> {
    let client = registration_client()?;
    let request = build_request(&client, api_base, payload, enrollment_token, device_token)?;
    execute_registration(
        &client,
        request,
        device_token.is_some(),
        enrollment_token.is_some(),
        false,
    )
    .await
}

/// Renew a registered machine after its short-lived access JWT has expired.
/// This credential is accepted only by the dedicated backend renewal endpoint.
pub async fn refresh_device(
    api_base: &str,
    payload: &DeviceRegistrationRequest,
    refresh_token: &str,
) -> Result<DeviceRegistrationResponse, &'static str> {
    if !valid_device_token(refresh_token) {
        return Err("设备凭据无效，请向管理员申请更新设备凭据");
    }
    let client = registration_client()?;
    let credential = Zeroizing::new(format!("Bearer {refresh_token}"));
    let request = build_authorized_request(
        &client,
        api_base,
        payload,
        "refresh",
        "X-Rdesk-Device-Refresh-Authorization",
        &credential,
    )?;
    execute_registration(&client, request, true, false, true).await
}

fn registration_client() -> Result<reqwest::Client, &'static str> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "无法初始化设备注册连接")
}

async fn execute_registration(
    client: &reqwest::Client,
    request: reqwest::Request,
    has_device_token: bool,
    has_enrollment_token: bool,
    requires_refresh: bool,
) -> Result<DeviceRegistrationResponse, &'static str> {
    let mut response = client
        .execute(request)
        .await
        .map_err(|_| "连接服务器失败，请稍后重试")?;
    match response.status().as_u16() {
        200..=299 => {}
        401 | 403 if has_device_token => return Err(DEVICE_CREDENTIAL_REJECTED),
        409 if has_enrollment_token => return Err("设备已登记，请向管理员申请更新设备凭据"),
        401 | 403 | 410 => return Err("设备登记码无效或已失效，请向管理员获取新登记码"),
        429 => return Err("注册请求过于频繁，请稍后重试"),
        _ => return Err("设备注册失败，请检查服务器配置后重试"),
    }
    const MAX_RESPONSE_BYTES: usize = 16 * 1024;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
    {
        return Err("服务器返回的设备登记响应无效");
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "服务器返回的设备登记响应无效")?
    {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err("服务器返回的设备登记响应无效");
        }
        body.extend_from_slice(&chunk);
    }
    let registration: DeviceRegistrationResponse =
        serde_json::from_slice(&body).map_err(|_| "服务器返回的设备登记响应无效")?;
    if registration.device_id.is_empty()
        || registration.device_name.is_empty()
        || registration.access_token.is_empty()
        || registration.device_id.len() > 64
        || registration.device_name.len() > 128
        || !valid_device_token(&registration.access_token)
        || registration
            .refresh_token
            .as_ref()
            .is_some_and(|token| !valid_device_token(token))
        || (requires_refresh && registration.refresh_token.is_none())
    {
        return Err("服务器返回的设备登记响应无效");
    }
    Ok(registration)
}

fn build_request(
    client: &reqwest::Client,
    api_base: &str,
    payload: &DeviceRegistrationRequest,
    enrollment_token: Option<&str>,
    device_token: Option<&str>,
) -> Result<reqwest::Request, &'static str> {
    let (header_name, credential) = match (enrollment_token, device_token) {
        (Some(token), None)
            if token.len() == 43
                && token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
        {
            ("X-Rdesk-Device-Enrollment", token.to_owned())
        }
        (None, Some(token)) if valid_device_token(token) => {
            ("X-Rdesk-Device-Authorization", format!("Bearer {token}"))
        }
        (None, None) => return Err("需要设备登记码，请向服务器管理员获取一次性登记码后注册"),
        _ => return Err("设备登记凭据无效，请重新输入管理员提供的登记码"),
    };
    let credential = Zeroizing::new(credential);
    build_authorized_request(
        client,
        api_base,
        payload,
        "register",
        header_name,
        &credential,
    )
}

fn valid_device_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 4096
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        && token.split('.').count() == 3
        && token.split('.').all(|part| !part.is_empty())
}

fn build_authorized_request(
    client: &reqwest::Client,
    api_base: &str,
    payload: &DeviceRegistrationRequest,
    endpoint: &str,
    header_name: &str,
    credential: &str,
) -> Result<reqwest::Request, &'static str> {
    use reqwest::{header::HeaderValue, Url};
    let mut url = Url::parse(api_base.trim()).map_err(|_| "服务器地址无效")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err("服务器地址不能包含凭据、查询参数或片段");
    }
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("服务器地址必须使用 HTTPS；本机调试可使用回环 HTTP 地址");
    }
    url.set_path(&format!(
        "{}/devices/{endpoint}",
        url.path().trim_end_matches('/')
    ));
    let mut header = HeaderValue::from_str(&credential).map_err(|_| "设备登记凭据无效")?;
    header.set_sensitive(true);
    client
        .post(url)
        .header(header_name, header)
        .json(payload)
        .build()
        .map_err(|_| "设备登记请求无效")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_optional_long_lived_device_credential() {
        let response: DeviceRegistrationResponse = serde_json::from_str(
            r#"{"device_id":"0123456789","device_name":"Office","access_token":"access.jwt.token","refresh_token":"refresh.jwt.token"}"#,
        ).unwrap();
        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["refresh_token"], "refresh.jwt.token");
        let debug = format!("{response:?}");
        assert!(!debug.contains("refresh.jwt.token") && !debug.contains("access.jwt.token"));
        let legacy: DeviceRegistrationResponse = serde_json::from_str(
            r#"{"device_id":"123456789","device_name":"Old","access_token":"access.jwt.token"}"#,
        )
        .unwrap();
        assert!(legacy.refresh_token.is_none());
    }

    #[tokio::test]
    async fn long_lived_renewal_uses_only_the_dedicated_endpoint_and_header() {
        let (base, captured) = fake_server("200 OK", r#"{"device_id":"0123456789","device_name":"Office","access_token":"access.jwt.token","refresh_token":"next.refresh.token"}"#, "").await;
        let response = refresh_device(&base, &payload(), "old.refresh.token")
            .await
            .unwrap();
        assert_eq!(
            response.refresh_token.as_deref(),
            Some("next.refresh.token")
        );
        let request = captured.await.unwrap();
        assert!(request.starts_with("POST /api/v1/devices/refresh HTTP/1.1\r\n"));
        assert!(
            request.contains("x-rdesk-device-refresh-authorization: Bearer old.refresh.token\r\n")
        );
        assert!(
            !request.contains("x-rdesk-device-authorization:")
                && !request.contains("x-rdesk-device-enrollment:")
        );
        assert!(request.contains("serial/with spaces"));
    }

    #[tokio::test]
    async fn renewal_rejects_missing_malformed_and_oversized_credentials() {
        let cases = [
            r#"{"device_id":"0123456789","device_name":"Office","access_token":"access.jwt.token"}"#.to_owned(),
            r#"{"device_id":"0123456789","device_name":"Office","access_token":"access.jwt.token","refresh_token":"not-a-jwt"}"#.to_owned(),
            format!(r#"{{"device_id":"0123456789","device_name":"Office","access_token":"access.jwt.token","refresh_token":"{}.jwt.token"}}"#, "a".repeat(4097)),
        ];
        for body in cases {
            let (base, captured) = fake_server("200 OK", &body, "").await;
            assert_eq!(
                refresh_device(&base, &payload(), "old.refresh.token")
                    .await
                    .unwrap_err(),
                "服务器返回的设备登记响应无效"
            );
            captured.await.unwrap();
        }
        for token in [
            "",
            "only-two.parts",
            "a..b",
            "a.b.c\r\nsecret",
            "秘密.a.b",
            &"a".repeat(4097),
        ] {
            assert!(
                refresh_device("https://example.com/api/v1", &payload(), token)
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn renewal_rejects_redirects_and_secret_error_bodies() {
        for (status, extra, expected) in [
            (
                "302 Found",
                "Location: https://example.com/credential-sink\r\n",
                "设备注册失败，请检查服务器配置后重试",
            ),
            (
                "401 Unauthorized",
                "",
                "设备凭据已失效，请向管理员申请更新设备凭据",
            ),
        ] {
            let (base, captured) = fake_server(status, "private-refresh-token", extra).await;
            assert_eq!(
                refresh_device(&base, &payload(), "old.refresh.token")
                    .await
                    .unwrap_err(),
                expected
            );
            captured.await.unwrap();
        }
        for base in [
            "http://example.com/api/v1",
            "https://user:secret@example.com/api/v1",
            "https://example.com/api/v1?token=secret",
        ] {
            assert!(refresh_device(base, &payload(), "old.refresh.token")
                .await
                .is_err());
        }
        let request = build_authorized_request(
            &registration_client().unwrap(),
            "https://example.com/api/v1",
            &payload(),
            "refresh",
            "X-Rdesk-Device-Refresh-Authorization",
            "Bearer old.refresh.token",
        )
        .unwrap();
        assert!(request.headers()["X-Rdesk-Device-Refresh-Authorization"].is_sensitive());
    }

    #[tokio::test]
    async fn ignores_environment_proxies_for_device_credentials() {
        const CHILD_FLAG: &str = "MRD_REGISTRATION_PROXY_TEST_CHILD";
        if std::env::var_os(CHILD_FLAG).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::ignores_environment_proxies_for_device_credentials",
                ])
                .env(CHILD_FLAG, "1")
                .env("HTTP_PROXY", "http://127.0.0.1:1")
                .env("ALL_PROXY", "http://127.0.0.1:1")
                .env("NO_PROXY", "")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }
        let (base, captured) = fake_server("200 OK", r#"{"device_id":"123456789","device_name":"Office PC","access_token":"device.jwt.token"}"#, "").await;
        assert!(
            register_device(&base, &payload(), Some(&"a".repeat(43)), None)
                .await
                .is_ok()
        );
        captured.await.unwrap();
        let (base, captured) = fake_server("200 OK", r#"{"device_id":"0123456789","device_name":"Office PC","access_token":"device.jwt.token","refresh_token":"refresh.jwt.token"}"#, "").await;
        assert!(refresh_device(&base, &payload(), "old.refresh.token")
            .await
            .is_ok());
        captured.await.unwrap();
    }

    async fn fake_server(
        status: &str,
        body: &str,
        extra_headers: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}", body.len());
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 1024];
            loop {
                let count = stream.read(&mut chunk).await.unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
                let text = String::from_utf8_lossy(&bytes);
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(bytes).unwrap()
        });
        (base, task)
    }

    #[tokio::test]
    async fn sends_enrollment_to_the_configured_endpoint() {
        let (base, captured) = fake_server("200 OK", r#"{"device_id":"123456789","device_name":"Office PC","access_token":"device.jwt.token"}"#, "").await;
        let token = "a".repeat(43);
        let result = register_device(&base, &payload(), Some(&token), None)
            .await
            .unwrap();
        assert_eq!(result.device_id, "123456789");
        let request = captured.await.unwrap();
        assert!(request.starts_with("POST /api/v1/devices/register HTTP/1.1\r\n"));
        assert!(request.contains(&format!("x-rdesk-device-enrollment: {token}\r\n")));
        assert!(request.contains("serial/with spaces"));
        assert!(request.contains(r#""cpu_info":"CPU model""#));
        assert!(request.contains(r#""total_memory_mb":8192"#));
        assert!(!request.contains("?"));
    }

    #[tokio::test]
    async fn rejects_http_errors_without_exposing_the_backend_body() {
        let (base, captured) = fake_server("401 Unauthorized", "sensitive-secret-token", "").await;
        assert_eq!(
            register_device(&base, &payload(), None, Some("device.jwt.token"))
                .await
                .err()
                .unwrap(),
            "设备凭据已失效，请向管理员申请更新设备凭据"
        );
        captured.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_oversized_registration_credentials_without_exposing_the_body() {
        let body = format!(
            r#"{{"device_id":"0123456789","device_name":"Office","access_token":"{}"}}"#,
            "sensitive".repeat(3000)
        );
        let (base, captured) = fake_server("200 OK", &body, "").await;
        let result = register_device(&base, &payload(), Some(&"a".repeat(43)), None).await;
        assert_eq!(result.err(), Some("服务器返回的设备登记响应无效"));
        captured.await.unwrap();
    }

    #[tokio::test]
    async fn does_not_follow_registration_redirects() {
        let (base, captured) = fake_server(
            "302 Found",
            "",
            "Location: https://example.com/credential-sink\r\n",
        )
        .await;
        assert_eq!(
            register_device(&base, &payload(), Some(&"a".repeat(43)), None)
                .await
                .err()
                .unwrap(),
            "设备注册失败，请检查服务器配置后重试"
        );
        captured.await.unwrap();
    }

    #[tokio::test]
    async fn enrollment_conflicts_direct_the_user_to_credential_recovery() {
        let (base, captured) = fake_server("409 Conflict", "sensitive-secret-token", "").await;
        assert_eq!(
            register_device(&base, &payload(), Some(&"a".repeat(43)), None)
                .await
                .err()
                .unwrap(),
            "设备已登记，请向管理员申请更新设备凭据"
        );
        captured.await.unwrap();
    }

    fn payload() -> DeviceRegistrationRequest {
        DeviceRegistrationRequest {
            motherboard_serial: "serial/with spaces".into(),
            hostname: "office".into(),
            os_version: "Windows 11".into(),
            device_name: Some("Office PC".into()),
            cpu_info: Some("CPU model".into()),
            total_memory_mb: Some(8192),
            gpu_info: Some("[]".into()),
        }
    }

    #[test]
    fn enrollment_is_sent_in_a_sensitive_header_and_never_in_the_url() {
        let token = "a".repeat(43);
        let request = build_request(
            &reqwest::Client::new(),
            "https://example.com/api/v1/",
            &payload(),
            Some(&token),
            None,
        )
        .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://example.com/api/v1/devices/register"
        );
        assert_eq!(request.method(), reqwest::Method::POST);
        let header = &request.headers()["X-Rdesk-Device-Enrollment"];
        assert_eq!(header, token.as_str());
        assert!(header.is_sensitive());
        assert!(!request
            .headers()
            .contains_key("X-Rdesk-Device-Authorization"));
    }

    #[test]
    fn refresh_uses_device_authorization() {
        let request = build_request(
            &reqwest::Client::new(),
            "http://127.0.0.1:9530/api/v1",
            &payload(),
            None,
            Some("device.jwt.token"),
        )
        .unwrap();
        let header = &request.headers()["X-Rdesk-Device-Authorization"];
        assert_eq!(header, "Bearer device.jwt.token");
        assert!(header.is_sensitive());
        assert!(!request.headers().contains_key("X-Rdesk-Device-Enrollment"));
    }

    #[test]
    fn missing_enrollment_fails_before_any_network_request() {
        assert_eq!(
            build_request(
                &reqwest::Client::new(),
                "https://example.com/api/v1",
                &payload(),
                None,
                None
            )
            .unwrap_err(),
            "需要设备登记码，请向服务器管理员获取一次性登记码后注册"
        );
    }

    #[test]
    fn permits_only_https_or_loopback_http() {
        let token = "a".repeat(43);
        for base in [
            "https://example.com/api/v1",
            "http://localhost:9530/api/v1",
            "http://127.0.0.1:9530/api/v1",
            "http://[::1]:9530/api/v1",
        ] {
            assert!(
                build_request(
                    &reqwest::Client::new(),
                    base,
                    &payload(),
                    Some(&token),
                    None
                )
                .is_ok(),
                "{base}"
            );
        }
        for base in [
            "http://example.com/api/v1",
            "http://192.168.1.5/api/v1",
            "ftp://example.com/api/v1",
            "https://user:secret@example.com/api/v1",
            "https://example.com/api/v1?secret=token",
            "https://example.com/api/v1#secret",
            "invalid",
        ] {
            assert!(
                build_request(
                    &reqwest::Client::new(),
                    base,
                    &payload(),
                    Some(&token),
                    None
                )
                .is_err(),
                "{base}"
            );
        }
    }

    #[test]
    fn rejects_ambiguous_credentials_and_invalid_codes() {
        let client = reqwest::Client::new();
        let token = "a".repeat(43);
        assert!(build_request(
            &client,
            "https://example.com/api/v1",
            &payload(),
            Some(&token),
            Some("device.token")
        )
        .is_err());
        for token in ["", "abc", "a\r\nsecret", &"a".repeat(44)] {
            assert!(build_request(
                &client,
                "https://example.com/api/v1",
                &payload(),
                Some(token),
                None
            )
            .is_err());
        }
        assert!(build_request(
            &client,
            "https://example.com/api/v1",
            &payload(),
            None,
            Some("jwt\r\nsecret")
        )
        .is_err());
    }
}
