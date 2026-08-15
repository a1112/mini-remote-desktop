use mrd_proto::{BackendRole, DeviceId};
use std::{fmt, time::Duration};
use thiserror::Error;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_INITIAL_RECONNECT: Duration = Duration::from_millis(500);
const DEFAULT_MAX_RECONNECT: Duration = Duration::from_secs(30);

/// Validated configuration for one service-owned signaling connection.
#[derive(Clone)]
pub struct SignalingConfig {
    endpoint: Url,
    device_id: DeviceId,
    device_name: String,
    role: BackendRole,
    backend_device_token: Zeroizing<String>,
    server_device_id: DeviceId,
    trusted_server_key_id: Option<String>,
    connect_timeout: Duration,
    initial_reconnect: Duration,
    max_reconnect: Duration,
}

impl fmt::Debug for SignalingConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SignalingConfig")
            .field("endpoint", &self.endpoint)
            .field("device_id", &self.device_id)
            .field("device_name", &self.device_name)
            .field("role", &self.role)
            .field("backend_device_token", &"REDACTED")
            .field("server_device_id", &self.server_device_id)
            .field("trusted_server_key_id", &self.trusted_server_key_id)
            .field("connect_timeout", &self.connect_timeout)
            .field("initial_reconnect", &self.initial_reconnect)
            .field("max_reconnect", &self.max_reconnect)
            .finish()
    }
}

impl Drop for SignalingConfig {
    fn drop(&mut self) {
        self.device_name.zeroize();
    }
}

impl SignalingConfig {
    /// Build and validate an explicit signaling configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoint: &str,
        device_id: DeviceId,
        device_name: &str,
        role: BackendRole,
        backend_device_token: &str,
        server_device_id: DeviceId,
        trusted_server_key_id: Option<String>,
        connect_timeout: Duration,
        initial_reconnect: Duration,
        max_reconnect: Duration,
    ) -> Result<Self, SignalingConfigError> {
        let endpoint = Url::parse(endpoint).map_err(|_| SignalingConfigError::InvalidEndpoint)?;
        validate_endpoint(&endpoint)?;
        if device_id.0.is_empty()
            || device_id.0.len() > 256
            || device_id.0.chars().any(char::is_control)
            || device_name.is_empty()
            || device_name.len() > 128
            || device_name.contains('\0')
            || backend_device_token.is_empty()
            || backend_device_token.len() > 4_096
            || backend_device_token.contains('\0')
            || server_device_id.0.is_empty()
            || server_device_id.0.len() > 256
            || server_device_id.0.chars().any(char::is_control)
            || connect_timeout.is_zero()
            || connect_timeout > Duration::from_secs(120)
            || initial_reconnect.is_zero()
            || max_reconnect < initial_reconnect
            || max_reconnect > Duration::from_secs(300)
            || trusted_server_key_id.as_ref().is_some_and(|key| {
                key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(SignalingConfigError::InvalidValue);
        }
        Ok(Self {
            endpoint,
            device_id,
            device_name: device_name.to_owned(),
            role,
            backend_device_token: Zeroizing::new(backend_device_token.to_owned()),
            server_device_id,
            trusted_server_key_id: trusted_server_key_id.map(|key| key.to_ascii_lowercase()),
            connect_timeout,
            initial_reconnect,
            max_reconnect,
        })
    }

    /// Load optional configuration. Missing `MRD_SIGNAL_URL` disables WAN signaling.
    pub fn from_env(
        device_id: DeviceId,
        device_name: &str,
    ) -> Result<Option<Self>, SignalingConfigError> {
        let Some(endpoint) = env_optional("MRD_SIGNAL_URL") else {
            return Ok(None);
        };
        let mut token = env_required("MRD_SIGNAL_DEVICE_TOKEN")?;
        let server_device_id = DeviceId(
            env_optional("MRD_SIGNAL_SERVER_DEVICE_ID").unwrap_or_else(|| "signal-server".into()),
        );
        let trusted_server_key_id = env_optional("MRD_SIGNAL_SERVER_KEY_ID");
        let role = match env_optional("MRD_SIGNAL_ROLE")
            .unwrap_or_else(|| "agent".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "agent" => BackendRole::Agent,
            "controller" => BackendRole::Controller,
            _ => return Err(SignalingConfigError::InvalidRole),
        };
        let connect_timeout = env_duration_ms(
            "MRD_SIGNAL_CONNECT_TIMEOUT_MS",
            DEFAULT_CONNECT_TIMEOUT,
            100,
            120_000,
        )?;
        let initial_reconnect = env_duration_ms(
            "MRD_SIGNAL_RECONNECT_INITIAL_MS",
            DEFAULT_INITIAL_RECONNECT,
            50,
            60_000,
        )?;
        let max_reconnect = env_duration_ms(
            "MRD_SIGNAL_RECONNECT_MAX_MS",
            DEFAULT_MAX_RECONNECT,
            100,
            300_000,
        )?;
        let result = Self::new(
            &endpoint,
            device_id,
            device_name,
            role,
            &token,
            server_device_id,
            trusted_server_key_id,
            connect_timeout,
            initial_reconnect,
            max_reconnect,
        )
        .map(Some);
        token.zeroize();
        result
    }

    pub(crate) fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub(crate) fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub(crate) fn device_name(&self) -> &str {
        &self.device_name
    }

    pub(crate) fn role(&self) -> BackendRole {
        self.role.clone()
    }

    pub(crate) fn backend_device_token(&self) -> &str {
        &self.backend_device_token
    }

    pub(crate) fn server_device_id(&self) -> &DeviceId {
        &self.server_device_id
    }

    pub(crate) fn trusted_server_key_id(&self) -> Option<&str> {
        self.trusted_server_key_id.as_deref()
    }

    pub(crate) fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    pub(crate) fn initial_reconnect(&self) -> Duration {
        self.initial_reconnect
    }

    pub(crate) fn max_reconnect(&self) -> Duration {
        self.max_reconnect
    }
}

fn validate_endpoint(endpoint: &Url) -> Result<(), SignalingConfigError> {
    if endpoint.as_str().len() > 2_048
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(SignalingConfigError::InvalidEndpoint);
    }
    match endpoint.scheme() {
        "wss" => Ok(()),
        "ws" if endpoint.host_str().is_some_and(is_loopback_host) => Ok(()),
        "ws" => Err(SignalingConfigError::InsecureEndpoint),
        _ => Err(SignalingConfigError::InvalidEndpoint),
    }
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn env_optional(name: &'static str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn env_required(name: &'static str) -> Result<String, SignalingConfigError> {
    env_optional(name).ok_or(SignalingConfigError::Missing(name))
}

fn env_duration_ms(
    name: &'static str,
    default: Duration,
    minimum: u64,
    maximum: u64,
) -> Result<Duration, SignalingConfigError> {
    let Some(raw) = env_optional(name) else {
        return Ok(default);
    };
    let value = raw
        .parse::<u64>()
        .map_err(|_| SignalingConfigError::InvalidEnvironment(name))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(SignalingConfigError::InvalidEnvironment(name));
    }
    Ok(Duration::from_millis(value))
}

/// Fail-closed signaling configuration error.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignalingConfigError {
    #[error("required signaling environment variable is missing: {0}")]
    Missing(&'static str),
    #[error("signaling endpoint is invalid")]
    InvalidEndpoint,
    #[error("unencrypted signaling is allowed only on loopback")]
    InsecureEndpoint,
    #[error("signaling role is invalid")]
    InvalidRole,
    #[error("signaling configuration contains an invalid value")]
    InvalidValue,
    #[error("signaling environment variable is invalid: {0}")]
    InvalidEnvironment(&'static str),
}
