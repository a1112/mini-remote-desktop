//! Own-device binding only: the caller never selects the target or API address.

use super::{validate_api_url, PublicConnectionState, Registration};
use mrd_ipc::{PublicUserCredential, PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Client, Request, StatusCode,
};
use serde::Deserialize;
use std::time::Duration;
use zeroize::Zeroizing;

const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(20);

fn binding_client() -> Result<Client, &'static str> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| "无法初始化设备绑定连接")
}

fn binding_url(
    state: &PublicConnectionState,
    registration: &Registration,
    bind: bool,
) -> Result<String, &'static str> {
    let configured = state.api_url.read().map_err(|_| "设备服务器配置不可用")?;
    validate_api_url(&configured).map_err(|_| "设备服务器配置无效")?;
    if registration.api_url.trim_end_matches('/') != configured.trim_end_matches('/') {
        return Err("设备凭据与当前服务器不匹配，请联系管理员");
    }
    Ok(format!(
        "{}/devices/{}",
        configured.trim_end_matches('/'),
        if bind { "auto-bind" } else { "unbind" }
    ))
}

fn binding_request(
    client: &Client,
    endpoint: &str,
    device_id: &str,
    device_token: &str,
    user_token: &PublicUserCredential,
) -> Result<Request, &'static str> {
    let user_bearer = Zeroizing::new(format!("Bearer {}", user_token.secret()));
    let device_bearer = Zeroizing::new(format!("Bearer {device_token}"));
    let mut user_header =
        HeaderValue::from_str(user_bearer.as_str()).map_err(|_| "用户登录凭据无效，请重新登录")?;
    user_header.set_sensitive(true);
    let mut device_header =
        HeaderValue::from_str(device_bearer.as_str()).map_err(|_| "设备凭据无效，请联系管理员")?;
    device_header.set_sensitive(true);
    client
        .post(endpoint)
        .header(AUTHORIZATION, user_header)
        .header("X-Rdesk-Device-Authorization", device_header)
        .json(&serde_json::json!({ "device_id": device_id }))
        .build()
        .map_err(|_| "无法创建本机设备绑定请求")
}

async fn send_binding(client: &Client, request: Request) -> Result<(), &'static str> {
    let mut response = client
        .execute(request)
        .await
        .map_err(|_| "无法连接设备服务器，请稍后重试")?;
    // Never expose a backend error body: it can contain identity or credential data.
    if !response.status().is_success() {
        return Err(match response.status() {
            StatusCode::UNAUTHORIZED => {
                "登录或设备凭据已过期，请重新登录；仍失败时联系管理员更新设备凭据"
            }
            StatusCode::FORBIDDEN => "当前账户无权绑定或解绑此设备",
            StatusCode::NOT_FOUND => "服务器未找到当前设备，请联系管理员",
            StatusCode::CONFLICT => "设备绑定状态已变化，请重试",
            _ => "服务器拒绝设备绑定操作，请稍后重试",
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err("服务器设备绑定响应无效");
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "服务器设备绑定响应中断")?
    {
        if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
            return Err("服务器设备绑定响应无效");
        }
        body.extend_from_slice(&chunk);
    }
    #[derive(Deserialize)]
    struct BindingResult {
        success: bool,
    }
    match serde_json::from_slice::<BindingResult>(&body) {
        Ok(result) if result.success => Ok(()),
        _ => Err("服务器未确认设备绑定操作，请重试"),
    }
}

pub(crate) async fn change_device_binding(
    state: &PublicConnectionState,
    protocol_minor: u16,
    user_token: PublicUserCredential,
    bind: bool,
) -> Result<(), &'static str> {
    if protocol_minor != PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR {
        return Err("设备绑定协议不兼容，请更新桌面客户端和后台服务");
    }
    tokio::time::timeout(OPERATION_TIMEOUT, async {
        // Enrollment, renewal and binding all choose credentials under this lock.
        let _operation = state.operation.lock().await;
        let registration = state
            .registration
            .read()
            .map_err(|_| "本机设备登记状态不可用")?
            .clone()
            .ok_or("请先完成本机设备登记，再绑定账户")?;
        let endpoint = binding_url(state, &registration, bind)?;
        let client = binding_client()?;
        let request = binding_request(
            &client,
            &endpoint,
            &registration.device_id,
            &registration.access_token,
            &user_token,
        )?;
        send_binding(&client, request).await
    })
    .await
    .map_err(|_| "本机设备绑定操作超时，请稍后重试")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
    use std::sync::{Arc, Mutex};

    fn user() -> PublicUserCredential {
        PublicUserCredential::try_from("user.access.token".to_owned()).unwrap()
    }
    fn registration() -> Registration {
        Registration {
            device_id: "0123456789".into(),
            device_name: "fixture".into(),
            access_token: "device.secret.jwt".into(),
            refresh_token: None,
            api_url: super::super::DEFAULT_PUBLIC_API_URL.into(),
            machine_serial: "fixture".into(),
        }
    }

    #[test]
    fn binding_uses_only_the_configured_https_origin_and_derived_device() {
        let state = PublicConnectionState::default();
        let mut saved = registration();
        let client = binding_client().unwrap();
        for (bind, action) in [(true, "auto-bind"), (false, "unbind")] {
            let endpoint = binding_url(&state, &saved, bind).unwrap();
            assert_eq!(endpoint, format!("{}/devices/{action}", saved.api_url));
            let request = binding_request(
                &client,
                &endpoint,
                &saved.device_id,
                &saved.access_token,
                &user(),
            )
            .unwrap();
            assert_eq!(request.headers()[AUTHORIZATION], "Bearer user.access.token");
            assert_eq!(
                request.headers()["X-Rdesk-Device-Authorization"],
                "Bearer device.secret.jwt"
            );
            assert!(request.headers()[AUTHORIZATION].is_sensitive());
            assert!(request.headers()["X-Rdesk-Device-Authorization"].is_sensitive());
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(
                    request.body().unwrap().as_bytes().unwrap()
                )
                .unwrap(),
                serde_json::json!({"device_id":"0123456789"})
            );
            assert!(!format!("{request:?}").contains("device.secret.jwt"));
        }
        saved.api_url = "https://attacker.example/api".into();
        assert!(binding_url(&state, &saved, true).is_err());
        *state.api_url.write().unwrap() = "http://127.0.0.1/api".into();
        saved.api_url = "http://127.0.0.1/api".into();
        assert!(binding_url(&state, &saved, true).is_err());
    }

    #[tokio::test]
    async fn binding_requires_registration_and_explicit_supported_minor() {
        let state = PublicConnectionState::default();
        assert!(change_device_binding(&state, 0, user(), true)
            .await
            .unwrap_err()
            .contains("协议"));
        assert!(
            change_device_binding(&state, PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR, user(), true)
                .await
                .unwrap_err()
                .contains("登记")
        );
    }

    #[tokio::test]
    async fn binding_and_renewal_share_the_operation_lock_and_choose_current_identity() {
        let state = Arc::new(PublicConnectionState::default());
        let lock = state.operation.lock().await;
        let pending_state = state.clone();
        let pending = tokio::spawn(async move {
            change_device_binding(
                &pending_state,
                PUBLIC_DEVICE_BINDING_PROTOCOL_MINOR,
                user(),
                true,
            )
            .await
        });
        tokio::task::yield_now().await;
        assert!(!pending.is_finished());
        // A registration/refresh replaces identity while retaining the same lock.
        *state.registration.write().unwrap() = Some(registration());
        *state.api_url.write().unwrap() = "http://forbidden.example".into();
        drop(lock);
        assert_eq!(pending.await.unwrap(), Err("设备服务器配置无效"));
    }

    #[tokio::test]
    async fn real_http_sends_both_credentials_and_limits_response_without_following_redirects() {
        type Seen = Arc<Mutex<Vec<(HeaderMap, serde_json::Value)>>>;
        async fn accepted(
            State(seen): State<Seen>,
            headers: HeaderMap,
            Json(body): Json<serde_json::Value>,
        ) -> Json<serde_json::Value> {
            seen.lock().unwrap().push((headers, body));
            Json(serde_json::json!({ "success":true, "message":"do-not-return-this-body" }))
        }
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let router = Router::new()
            .route("/devices/auto-bind", post(accepted))
            .route("/devices/unbind", post(accepted))
            .route(
                "/redirect",
                post(|| async {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [("location", "/devices/auto-bind")],
                        "secret-body",
                    )
                }),
            )
            .route(
                "/huge",
                post(|| async { "x".repeat(MAX_RESPONSE_BYTES + 1) }),
            )
            .route(
                "/denied",
                post(|| async {
                    (
                        StatusCode::UNAUTHORIZED,
                        "device.secret.jwt user.access.token",
                    )
                }),
            )
            .route(
                "/false",
                post(|| async { Json(serde_json::json!({ "success":false })) }),
            )
            .with_state(seen.clone());
        // HTTP is permitted only in this private transport fixture. Production
        // validates HTTPS and exact saved/configured origin before building requests.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = binding_client().unwrap();
        for action in ["auto-bind", "unbind"] {
            let request = binding_request(
                &client,
                &format!("http://{address}/devices/{action}"),
                "0123456789",
                "device.secret.jwt",
                &user(),
            )
            .unwrap();
            assert_eq!(send_binding(&client, request).await, Ok(()));
        }
        assert_eq!(seen.lock().unwrap().len(), 2);
        for (headers, body) in seen.lock().unwrap().iter() {
            assert_eq!(headers[AUTHORIZATION], "Bearer user.access.token");
            assert_eq!(
                headers["X-Rdesk-Device-Authorization"],
                "Bearer device.secret.jwt"
            );
            assert_eq!(body, &serde_json::json!({ "device_id":"0123456789" }));
        }
        for route in ["redirect", "huge", "denied", "false"] {
            let request = binding_request(
                &client,
                &format!("http://{address}/{route}"),
                "0123456789",
                "device.secret.jwt",
                &user(),
            )
            .unwrap();
            let error = send_binding(&client, request).await.unwrap_err();
            assert!(!error.contains("secret"));
            assert!(!error.contains("token"));
        }
        assert_eq!(
            seen.lock().unwrap().len(),
            2,
            "redirect must not follow to the authorized endpoint"
        );
        server.abort();
    }
}
