//! Bounded, credential-redacted HTTP publication to the configured device origin.
use crate::temporary_access::TemporaryPublicationProof;
use mrd_ipc::TemporaryAccessStatus;
use reqwest::Client;

pub(crate) fn client() -> Result<Client, &'static str> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .map_err(|_| "temporary_http_unavailable")
}
fn bearer(token: &str) -> Result<reqwest::header::HeaderValue, &'static str> {
    let value = zeroize::Zeroizing::new(format!("Bearer {token}"));
    let mut header = reqwest::header::HeaderValue::from_str(value.as_str())
        .map_err(|_| "temporary_device_credential_invalid")?;
    header.set_sensitive(true);
    Ok(header)
}
async fn response(mut response: reqwest::Response) -> Result<TemporaryAccessStatus, &'static str> {
    if !response.status().is_success() {
        return Err("temporary_publication_rejected");
    }
    const LIMIT: usize = 16 * 1024;
    if response
        .content_length()
        .is_some_and(|length| length > LIMIT as u64)
    {
        return Err("temporary_response_invalid");
    }
    let mut body = zeroize::Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "temporary_response_interrupted")?
    {
        if chunk.len() > LIMIT.saturating_sub(body.len()) {
            return Err("temporary_response_invalid");
        }
        body.extend_from_slice(&chunk);
    }
    let status: TemporaryAccessStatus =
        serde_json::from_slice(body.as_ref()).map_err(|_| "temporary_response_invalid")?;
    if status.generation > i64::MAX as u64
        || status.ready && (!status.enabled || status.expires_at_ms.is_none())
        || status
            .reason
            .as_ref()
            .is_some_and(|reason| reason.len() > 128)
    {
        return Err("temporary_response_invalid");
    }
    Ok(status)
}
pub(crate) async fn metadata(
    client: &Client,
    endpoint: &str,
    token: &str,
) -> Result<TemporaryAccessStatus, &'static str> {
    response(
        client
            .get(endpoint)
            .header(reqwest::header::AUTHORIZATION, bearer(token)?)
            .send()
            .await
            .map_err(|_| "temporary_http_unavailable")?,
    )
    .await
}
pub(crate) async fn publish(
    client: &Client,
    endpoint: &str,
    token: &str,
    proof: &TemporaryPublicationProof,
) -> Result<TemporaryAccessStatus, &'static str> {
    response(
        client
            .post(endpoint)
            .header(reqwest::header::AUTHORIZATION, bearer(token)?)
            .json(proof)
            .send()
            .await
            .map_err(|_| "temporary_http_unavailable")?,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        routing::{get, post},
        Json, Router,
    };
    #[tokio::test]
    async fn real_http_publication_and_metadata_are_bounded_and_do_not_redirect() {
        let router=Router::new()
            .route("/temporary",get(||async{Json(serde_json::json!({"enabled":false,"ready":false,"generation":4,"expires_at_ms":null,"reason":"disabled"}))})
                .post(|headers:axum::http::HeaderMap,Json(body):Json<serde_json::Value>|async move {
                    assert_eq!(headers[reqwest::header::AUTHORIZATION],"Bearer synthetic-device-token");
                    assert_eq!(body["key_id"],"fixture-key");
                    Json(serde_json::json!({"enabled":true,"ready":true,"generation":5,"expires_at_ms":600000,"reason":null}))
                }))
            .route("/redirect",post(||async{(reqwest::StatusCode::TEMPORARY_REDIRECT,[("location","/temporary")],"private-token-body")}))
            .route("/huge",post(||async{"x".repeat(16385)}))
            .route("/denied",post(||async{(reqwest::StatusCode::UNAUTHORIZED,"private-token-body")}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = client().unwrap();
        assert_eq!(
            metadata(
                &client,
                &format!("http://{address}/temporary"),
                "synthetic-device-token"
            )
            .await
            .unwrap()
            .generation,
            4
        );
        let proof = TemporaryPublicationProof {
            key_id: "fixture-key".into(),
            public_key: "public-key".into(),
            access_json: "{}".into(),
            signature: "signature".into(),
        };
        assert_eq!(
            publish(
                &client,
                &format!("http://{address}/temporary"),
                "synthetic-device-token",
                &proof
            )
            .await
            .unwrap()
            .generation,
            5
        );
        for route in ["redirect", "huge", "denied"] {
            let error = publish(
                &client,
                &format!("http://{address}/{route}"),
                "synthetic-device-token",
                &proof,
            )
            .await
            .unwrap_err();
            assert!(!error.contains("private-token-body"));
        }
        server.abort();
    }
}
