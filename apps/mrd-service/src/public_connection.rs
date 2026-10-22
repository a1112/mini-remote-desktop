//! Resident ownership of public identity, credentials and connection health.
//!
//! The narrow management endpoint accepts enrollment capabilities only for the
//! configured HTTPS origin. The service chooses its machine identity and keeps
//! bearer credentials inside protected storage; a WebView never receives them.

use crate::{relay, signaling, wan_session, AppState};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use mrd_device_registration::{DeviceRegistrationRequest, DeviceRegistrationResponse};
use mrd_ipc::PublicServerStatus;
use mrd_proto::{BackendRole, DeviceId};
use mrd_store_sqlite::SecretProtector;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::{
    sync::{oneshot, Mutex, Notify},
    task::JoinHandle,
};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

mod binding;
mod initial;
pub(crate) mod temporary;
pub(crate) use binding::change_device_binding;

pub const DEFAULT_PUBLIC_API_URL: &str = "https://175.178.16.90/rdesk/api/v1";
const CONFIG_PURPOSE: &[u8] = b"MRD_PUBLIC_DEVICE_CREDENTIAL_V1\0";
const MAX_CONFIG_BYTES: usize = 64 * 1024;

/// Shared credential copies are short-lived and zeroed after each HTTP request.
pub struct DeviceCredential(RwLock<Zeroizing<String>>);
impl DeviceCredential {
    pub fn new(token: &str) -> Self {
        Self(RwLock::new(Zeroizing::new(token.to_owned())))
    }
    pub(crate) fn snapshot(&self) -> Zeroizing<String> {
        Zeroizing::new(
            self.0
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_str()
                .to_owned(),
        )
    }
    fn replace(&self, token: &str) {
        *self
            .0
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Zeroizing::new(token.to_owned());
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    device_id: String,
    device_name: String,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    api_url: String,
    #[serde(default)]
    machine_serial: String,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

struct ProtectedConfig {
    path: PathBuf,
    protector: Arc<dyn SecretProtector>,
    purpose: Vec<u8>,
    machine_key_id: String,
}
impl ProtectedConfig {
    fn load(&self) -> Result<Option<Registration>> {
        let metadata = match std::fs::symlink_metadata(&self.path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= MAX_CONFIG_BYTES as u64,
            "public credential file is invalid"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            anyhow::ensure!(
                metadata.mode() & 0o077 == 0
                    && metadata.nlink() == 1
                    && metadata.uid() == unsafe { libc::geteuid() },
                "public credential permissions are invalid"
            );
        }
        let protected = std::fs::read(&self.path)?;
        let plaintext = self
            .protector
            .unprotect(&self.purpose, &protected)
            .map_err(|_| anyhow!("public credential protection failed"))?;
        let value: Registration = serde_json::from_slice(plaintext.as_ref())
            .map_err(|_| anyhow!("public credential format is invalid"))?;
        validate_api_url(&value.api_url)?;
        anyhow::ensure!(
            !value.device_id.is_empty()
                && value.device_id.len() <= 64
                && !value.access_token.is_empty()
                && value.access_token.len() <= 4096
                && value
                    .refresh_token
                    .as_ref()
                    .is_none_or(|token| !token.is_empty() && token.len() <= 4096),
            "public credential identity is invalid"
        );
        Ok(Some(value))
    }
    fn save(&self, value: &Registration) -> Result<()> {
        use std::io::Write;
        let plaintext = Zeroizing::new(serde_json::to_vec(value)?);
        let protected = self
            .protector
            .protect(&self.purpose, &plaintext)
            .map_err(|_| anyhow!("public credential protection failed"))?;
        anyhow::ensure!(
            protected.len() <= MAX_CONFIG_BYTES,
            "public credential exceeds size limit"
        );
        let temporary = self.path.with_extension(format!("{}.tmp", uuid_suffix()?));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        let written = (|| {
            file.write_all(&protected)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &self.path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written.context("public credential save failed")
    }
}

fn uuid_suffix() -> Result<String> {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| anyhow!("random source unavailable"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Readable status and serialized enrollment, independent of UI lifetime.
pub struct PublicConnectionState {
    pub(crate) temporary: Arc<Mutex<crate::temporary_access::TemporaryAccessState>>,
    pub(crate) temporary_operation: Mutex<()>,
    pub(crate) temporary_epoch: Arc<crate::temporary_access::TemporaryOperationEpoch>,
    pub(crate) temporary_auth_version: std::sync::atomic::AtomicU64,
    registration: RwLock<Option<Registration>>,
    persistence: RwLock<Option<ProtectedConfig>>,
    signing_counter: std::sync::Mutex<Option<Arc<signaling::PersistentSignalingCounter>>>,
    api_url: RwLock<String>,
    machine_serial: RwLock<String>,
    api_reachable: RwLock<Option<bool>>,
    last_error: RwLock<Option<String>>,
    operation: Mutex<()>,
    changed: Notify,
}
impl Default for PublicConnectionState {
    fn default() -> Self {
        let temporary_epoch = Arc::new(crate::temporary_access::TemporaryOperationEpoch::default());
        Self {
            temporary: Arc::new(Mutex::new(
                crate::temporary_access::TemporaryAccessState::default()
                    .with_operation_epoch(temporary_epoch.clone()),
            )),
            temporary_operation: Mutex::new(()),
            temporary_epoch,
            temporary_auth_version: std::sync::atomic::AtomicU64::new(0),
            registration: RwLock::new(None),
            persistence: RwLock::new(None),
            signing_counter: std::sync::Mutex::new(None),
            api_url: RwLock::new(DEFAULT_PUBLIC_API_URL.to_owned()),
            machine_serial: RwLock::new(String::new()),
            api_reachable: RwLock::new(None),
            last_error: RwLock::new(None),
            operation: Mutex::new(()),
            changed: Notify::new(),
        }
    }
}
impl PublicConnectionState {
    pub fn configure_persistence(
        &self,
        directory: PathBuf,
        protector: Arc<dyn SecretProtector>,
        machine_key_id: &str,
    ) -> Result<()> {
        let api_url = std::env::var("MRD_PUBLIC_API_URL")
            .unwrap_or_else(|_| DEFAULT_PUBLIC_API_URL.to_owned());
        validate_api_url(&api_url)?;
        let mut purpose = CONFIG_PURPOSE.to_vec();
        purpose.extend_from_slice(machine_key_id.as_bytes());
        let config = ProtectedConfig {
            path: directory.join("public-device-v1.protected"),
            protector,
            purpose,
            machine_key_id: machine_key_id.to_owned(),
        };
        let saved = config.load()?;
        let serial = match &saved {
            Some(saved) if !saved.machine_serial.is_empty() => saved.machine_serial.clone(),
            Some(_) => format!("mrd-machine-key:{machine_key_id}"),
            None => mrd_device_registration::stable_machine_identity(Some(machine_key_id))
                .map_err(|_| anyhow!("stable machine identity unavailable"))?,
        };
        if let Some(saved) = &saved {
            anyhow::ensure!(
                saved.api_url.trim_end_matches('/') == api_url.trim_end_matches('/'),
                "public credential server differs from configured origin"
            );
        }
        *self.api_url.write().unwrap() = api_url;
        *self.machine_serial.write().unwrap() = serial;
        *self.registration.write().unwrap() = saved;
        *self.persistence.write().unwrap() = Some(config);
        Ok(())
    }
    pub(crate) fn signaling_counter(
        &self,
    ) -> Result<Option<Arc<signaling::PersistentSignalingCounter>>> {
        let mut cached = self
            .signing_counter
            .lock()
            .map_err(|_| anyhow!("protected signaling counter is unavailable"))?;
        if let Some(counter) = cached.as_ref() {
            return Ok(Some(counter.clone()));
        }
        let persistence = self
            .persistence
            .read()
            .map_err(|_| anyhow!("protected signaling counter is unavailable"))?;
        let Some(config) = persistence.as_ref() else {
            return Ok(None);
        };
        let directory = config
            .path
            .parent()
            .ok_or_else(|| anyhow!("protected signaling counter directory is invalid"))?;
        let counter = Arc::new(
            signaling::PersistentSignalingCounter::open(
                directory,
                config.protector.clone(),
                &config.machine_key_id,
            )
            .map_err(|_| anyhow!("protected signaling counter is unavailable or invalid"))?,
        );
        *cached = Some(counter.clone());
        Ok(Some(counter))
    }
    pub(crate) fn device_auth_version(&self) -> Option<u64> {
        self.registration()
            .and_then(|saved| crate::temporary_access::decoded_auth_version(&saved.access_token))
    }
    fn registration(&self) -> Option<Registration> {
        self.registration.read().unwrap().clone()
    }
    fn save(&self, value: Registration) -> Result<()> {
        let persistence = self.persistence.read().unwrap();
        let store = persistence
            .as_ref()
            .ok_or_else(|| anyhow!("protected public credential storage is unavailable"))?;
        store.save(&value)?;
        *self.registration.write().unwrap() = Some(value);
        self.changed.notify_one();
        Ok(())
    }
    pub fn snapshot(&self, signaling: &signaling::SignalingRuntimeSnapshot) -> PublicServerStatus {
        use signaling::SignalingConnectionState as Phase;
        let registration = self.registration.read().unwrap();
        PublicServerStatus {
            service_running: true,
            api_url: Some(self.api_url.read().unwrap().clone()),
            api_reachable: *self.api_reachable.read().unwrap(),
            device_registered: registration.is_some(),
            device_id: registration.as_ref().map(|value| value.device_id.clone()),
            device_name: registration.as_ref().map(|value| value.device_name.clone()),
            signaling_state: match signaling.state {
                Phase::Disabled => "disabled",
                Phase::Connecting => "connecting",
                Phase::Authenticated => "authenticated",
                Phase::Backoff => "backoff",
                Phase::Stopped => "stopped",
            }
            .into(),
            reconnect_attempt: signaling.reconnect_attempt,
            last_connected_at_ms: signaling.last_connected_at_ms,
            last_error: self
                .last_error
                .read()
                .unwrap()
                .clone()
                .or_else(|| signaling.last_error.clone()),
        }
    }
}

fn validate_api_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| anyhow!("invalid public API address"))?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && value.len() <= 2048,
        "public API address must be HTTPS without credentials or query parameters"
    );
    Ok(url)
}

fn machine_payload(state: &AppState, name: String) -> DeviceRegistrationRequest {
    let hostname = sysinfo::System::host_name().unwrap_or_else(|| "Rdesk Device".into());
    DeviceRegistrationRequest {
        motherboard_serial: state
            .public_connection
            .machine_serial
            .read()
            .unwrap()
            .clone(),
        hostname,
        os_version: sysinfo::System::long_os_version()
            .unwrap_or_else(|| std::env::consts::OS.into()),
        device_name: Some(name),
        cpu_info: None,
        total_memory_mb: None,
        gpu_info: None,
    }
}

/// The enrollment capability authorizes only this already-selected machine and origin.
pub async fn enroll(
    state: &Arc<AppState>,
    enrollment_token: String,
    name: String,
) -> Result<PublicServerStatus, &'static str> {
    let token = Zeroizing::new(enrollment_token);
    if token.len() != 43
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        || name.trim().is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
    {
        return Err("请输入有效的一次性设备登记码和设备名称");
    }
    let _guard = state.public_connection.operation.lock().await;
    if state.public_connection.registration().is_some() {
        return Err("本机已有公网设备码，请使用凭据恢复保留原设备身份");
    }
    let api_url = state.public_connection.api_url.read().unwrap().clone();
    if state
        .public_connection
        .persistence
        .read()
        .unwrap()
        .is_none()
    {
        return Err("后台安全存储尚未就绪");
    }
    let registration = mrd_device_registration::register_device(
        &api_url,
        &machine_payload(state, name),
        Some(&token),
        None,
    )
    .await?;
    apply_registration(state, &api_url, registration).await?;
    Ok(state
        .public_connection
        .snapshot(&state.signaling_status.snapshot()))
}

pub async fn recover(
    state: &Arc<AppState>,
    credential: String,
) -> Result<PublicServerStatus, &'static str> {
    let credential = Zeroizing::new(credential);
    if credential.len() > 4096
        || credential.split('.').count() != 3
        || !credential
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err("设备恢复凭据无效");
    }
    let _guard = state.public_connection.operation.lock().await;
    let saved = state
        .public_connection
        .registration()
        .ok_or("本机尚未登记，请输入一次性设备登记码")?;
    let registration = mrd_device_registration::register_device(
        &saved.api_url,
        &machine_payload(state, saved.device_name.clone()),
        None,
        Some(&credential),
    )
    .await?;
    if registration.device_id != saved.device_id {
        return Err("设备恢复身份不匹配");
    }
    apply_registration(state, &saved.api_url, registration).await?;
    Ok(state
        .public_connection
        .snapshot(&state.signaling_status.snapshot()))
}

async fn apply_registration(
    state: &Arc<AppState>,
    api_url: &str,
    value: DeviceRegistrationResponse,
) -> Result<(), &'static str> {
    apply_registration_inner(state, api_url, value, false).await
}

// Only a successful durable machine-key challenge may restore a historical
// nine-digit code when the local credential file is absent. Ordinary OTP
// allocation retains its ten-digit requirement, and an existing code never changes.
async fn apply_self_registration(
    state: &Arc<AppState>,
    api_url: &str,
    value: DeviceRegistrationResponse,
) -> Result<(), &'static str> {
    if state.public_connection.registration().is_some()
        || !matches!(value.device_id.len(), 9 | 10)
        || !value.device_id.bytes().all(|byte| byte.is_ascii_digit())
        || value.refresh_token.as_deref().is_none_or(str::is_empty)
    {
        return Err("public_self_enrollment_response_invalid");
    }
    apply_registration_inner(state, api_url, value, true).await
}

async fn apply_registration_inner(
    state: &Arc<AppState>,
    api_url: &str,
    mut value: DeviceRegistrationResponse,
    verified_self_enrollment: bool,
) -> Result<(), &'static str> {
    let previous = state.public_connection.registration();
    if previous
        .as_ref()
        .is_some_and(|saved| saved.device_id != value.device_id)
    {
        return Err("设备恢复身份不匹配");
    }
    if value.device_id.len() != 10 || !value.device_id.bytes().all(|byte| byte.is_ascii_digit()) {
        // Historical IDs are acceptable only when refreshing an existing identity.
        if state
            .public_connection
            .registration()
            .as_ref()
            .is_none_or(|saved| saved.device_id != value.device_id)
            && !(verified_self_enrollment
                && value.device_id.len() == 9
                && value.device_id.bytes().all(|byte| byte.is_ascii_digit()))
        {
            value.access_token.zeroize();
            return Err("服务器未返回有效的10位设备码");
        }
    }
    let saved = Registration {
        device_id: value.device_id.clone(),
        device_name: value.device_name.clone(),
        access_token: std::mem::take(&mut value.access_token),
        refresh_token: value.refresh_token.take().or_else(|| {
            previous
                .as_ref()
                .filter(|saved| {
                    saved.api_url == api_url
                        && saved.device_id == value.device_id
                        && saved.machine_serial
                            == *state.public_connection.machine_serial.read().unwrap()
                })
                .and_then(|saved| saved.refresh_token.clone())
        }),
        api_url: api_url.to_owned(),
        machine_serial: state
            .public_connection
            .machine_serial
            .read()
            .unwrap()
            .clone(),
    };
    state
        .public_connection
        .save(saved)
        .map_err(|_| "无法安全保存设备凭据，请使用同一登记码重试")?;
    state.devices.lock().await.register(
        DeviceId(std::mem::take(&mut value.device_id)),
        std::mem::take(&mut value.device_name),
    );
    *state.public_connection.api_reachable.write().unwrap() = Some(true);
    *state.public_connection.last_error.write().unwrap() = None;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bootstrap {
    signaling_url: String,
    signaling_server_device_id: String,
    signaling_server_key_id: Option<String>,
    relay_directory_url: String,
    relay_directory_keys: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
#[error("public server responded but its connection configuration is unavailable")]
struct PublicConfigurationUnavailable;

async fn fetch_bootstrap(api_url: &str) -> Result<Bootstrap> {
    let base = validate_api_url(api_url)?;
    let endpoint = format!("{}/public/connection-config", api_url.trim_end_matches('/'));
    let response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?
        .get(endpoint)
        .send()
        .await?;
    validate_bootstrap_response(response, base)
        .await
        .map_err(|error| error.context(PublicConfigurationUnavailable))
}

async fn validate_bootstrap_response(response: reqwest::Response, base: Url) -> Result<Bootstrap> {
    use futures_util::StreamExt;
    anyhow::ensure!(
        response.status().is_success(),
        "public server configuration unavailable"
    );
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        anyhow::ensure!(
            body.len() + chunk.len() <= MAX_CONFIG_BYTES,
            "public server configuration too large"
        );
        body.extend_from_slice(&chunk);
    }
    let value: Bootstrap = serde_json::from_slice(&body)?;
    let signal = Url::parse(&value.signaling_url)?;
    let relay = validate_api_url(&value.relay_directory_url)?;
    anyhow::ensure!(
        signal.scheme() == "wss"
            && signal.username().is_empty()
            && signal.password().is_none()
            && signal.query().is_none()
            && signal.fragment().is_none()
            && signal.host_str() == base.host_str()
            && signal.port_or_known_default() == base.port_or_known_default()
            && relay.origin() == base.origin(),
        "public configuration changed trusted origin"
    );
    anyhow::ensure!(
        !value.relay_directory_keys.is_empty() && value.relay_directory_keys.len() <= 8,
        "relay verification keys unavailable"
    );
    Ok(value)
}

struct Stack {
    signaling: Option<signaling::SignalingTask>,
    responder: Option<relay::ServiceRelayResponderTask>,
    wan: Option<wan_session::service::ServiceWanSessionTask>,
}

/// A process owner cannot be rebound after installation began. The resident
/// must terminate so its supervisor can create fresh, authenticated owners.
#[derive(Debug, thiserror::Error)]
#[error("public runtime installation failed; process restart required")]
struct StackInstallFailed;

impl Stack {
    async fn shutdown(self) {
        if let Some(task) = self.wan {
            task.shutdown().await;
        }
        if let Some(task) = self.responder {
            task.shutdown().await;
        }
        if let Some(task) = self.signaling {
            task.shutdown().await;
        }
    }
}

async fn start_stack(
    state: &Arc<AppState>,
    registration: &Registration,
    bootstrap: Bootstrap,
    credential: Arc<DeviceCredential>,
) -> Result<Stack> {
    let keys: BTreeMap<String, Vec<u8>> = bootstrap
        .relay_directory_keys
        .into_iter()
        .map(|(key, value)| STANDARD.decode(value).map(|bytes| (key, bytes)))
        .collect::<std::result::Result<_, _>>()?;
    let backend: Arc<dyn wan_session::backend::WanSessionBackend> =
        Arc::new(wan_session::backend::HttpWanSessionBackend::new(
            wan_session::config::WanSessionBackendConfig::new(
                &registration.api_url,
                &registration.access_token,
                keys.clone(),
                Duration::from_secs(5),
                2 * 1024 * 1024,
                3,
            )?
            .with_device_credential(credential.clone()),
        )?);
    let directory = Arc::new(relay::RelayDirectoryClient::new(
        relay::RelayClientConfig::new(
            &bootstrap.relay_directory_url,
            &registration.access_token,
            keys,
            Duration::from_secs(5),
            32,
        )?
        .with_device_credential(credential.clone()),
    )?);
    let role = BackendRole::Peer;
    let mut config = signaling::SignalingConfig::new(
        &bootstrap.signaling_url,
        DeviceId(registration.device_id.clone()),
        &registration.device_name,
        role,
        &registration.access_token,
        DeviceId(bootstrap.signaling_server_device_id),
        bootstrap.signaling_server_key_id,
        Duration::from_secs(10),
        Duration::from_millis(500),
        Duration::from_secs(30),
    )?
    .with_backend_api_url(&registration.api_url)?
    .with_device_credential(credential);
    let counter = state
        .public_connection
        .signaling_counter()?
        .ok_or_else(|| anyhow!("protected signaling counter storage is not configured"))?;
    config = config.with_counter_allocator(counter);
    let executor: Arc<dyn relay::RelayMigrationExecutor> = Arc::new(
        relay::ServiceRelayMigrationExecutor::new(state.clone(), Duration::from_secs(20))?,
    );
    let provider: Arc<dyn relay::RelayAccessProvider> = directory.clone();
    let input: Arc<dyn relay::RelayInputBarrier> =
        Arc::new(relay::ServiceRelayInputBarrier::new(state));
    let coordinator = Arc::new(relay::RelayFailoverCoordinator::new(
        provider,
        executor,
        input,
        Arc::new(relay::SystemRelayClock),
        Duration::from_secs(3),
    )?);
    // Configuration above is retryable. Bindings below, including the signaling
    // receiver, are process-wide and deliberately one-shot. Retrying a partial
    // install on the same AppState would leave the service permanently frozen.
    let mut stack = Stack {
        signaling: None,
        responder: None,
        wan: None,
    };
    let installed: Result<()> = async {
        state
            .bind_wan_session_backend(backend.clone())
            .map_err(anyhow::Error::msg)?;
        state
            .bind_relay_directory_client(directory)
            .map_err(anyhow::Error::msg)?;
        state
            .bind_relay_failover_coordinator(coordinator.clone())
            .map_err(anyhow::Error::msg)?;
        stack.signaling = Some(signaling::spawn(config, state.clone())?);
        stack.responder = Some(relay::spawn_relay_migration_responder(
            state.clone(),
            coordinator,
        ));
        stack.wan = Some(
            wan_session::service::bind_and_spawn_wan_session_service(state.clone(), backend)
                .await?,
        );
        Ok(())
    }
    .await;
    if let Err(error) = installed {
        stack.shutdown().await;
        return Err(error.context(StackInstallFailed));
    }
    Ok(stack)
}

pub struct PublicConnectionTask {
    temporary: temporary::TemporaryTask,
    stop: oneshot::Sender<()>,
    join: JoinHandle<Result<()>>,
}
impl PublicConnectionTask {
    pub async fn shutdown(self) -> Result<()> {
        let _ = self.stop.send(());
        self.temporary.shutdown().await;
        self.join
            .await
            .context("public connection supervisor failed")?
    }
}

/// Always starts a supervisor, including while the device is waiting for enrollment.
pub async fn spawn(state: Arc<AppState>) -> Result<PublicConnectionTask> {
    let temporary = temporary::spawn(state.clone());
    let (stop, mut stopping) = oneshot::channel();
    if let Some(saved) = state.public_connection.registration() {
        state
            .devices
            .lock()
            .await
            .register(DeviceId(saved.device_id.clone()), saved.device_name.clone());
    }
    let join = tokio::spawn(async move {
        let mut terminal_failure = None;
        let mut stack: Option<Stack> = None;
        let mut credential: Option<Arc<DeviceCredential>> = None;
        let mut last_refresh: Option<tokio::time::Instant> = None;
        let mut initial_retry = initial::EnrollmentRetry::default();
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! { _ = &mut stopping => break, _ = ticker.tick() => {}, _ = state.public_connection.changed.notified() => {} }
            if stack
                .as_ref()
                .and_then(|value| value.signaling.as_ref())
                .is_some_and(signaling::SignalingTask::is_finished)
            {
                *state.public_connection.last_error.write().unwrap() =
                    Some("public_runtime_start_failed".into());
                let mode = mrd_ipc::ShutdownMode::Graceful;
                if state.shutdown.request(mode.clone()).is_ok() {
                    state.shutdown.acknowledge(mode);
                }
                terminal_failure = Some(anyhow!(
                    "public signaling task exited; process restart required"
                ));
                break;
            }
            let api_url = state.public_connection.api_url.read().unwrap().clone();
            let fetched = tokio::select! { _ = &mut stopping => break, result = fetch_bootstrap(&api_url) => result };
            *state.public_connection.api_reachable.write().unwrap() =
                Some(fetched.as_ref().map_or_else(
                    |error| error.is::<PublicConfigurationUnavailable>(),
                    |_| true,
                ));
            let bootstrap = match fetched {
                Ok(value) => value,
                Err(error) => {
                    *state.public_connection.last_error.write().unwrap() = Some(
                        if error.is::<PublicConfigurationUnavailable>() {
                            "public_configuration_unavailable"
                        } else {
                            "public_api_unreachable"
                        }
                        .into(),
                    );
                    continue;
                }
            };
            {
                let mut error = state.public_connection.last_error.write().unwrap();
                if matches!(
                    error.as_deref(),
                    Some("public_api_unreachable" | "public_configuration_unavailable")
                ) {
                    *error = None;
                }
            }
            if state.public_connection.registration().is_none() {
                if !initial_retry.can_attempt(tokio::time::Instant::now()) {
                    continue;
                }
                let enrolled = tokio::select! {
                    biased;
                    _ = &mut stopping => break,
                    result = initial::ensure_registration(&state,&api_url) => result,
                };
                match enrolled {
                    Ok(created) => {
                        initial_retry.succeeded();
                        if created {
                            last_refresh = Some(tokio::time::Instant::now());
                        }
                    }
                    Err(code) => {
                        initial_retry.failed(tokio::time::Instant::now(), code);
                        *state.public_connection.last_error.write().unwrap() = Some(code.into());
                        continue;
                    }
                }
            }
            let _operation = tokio::select! {
                _ = &mut stopping => break,
                guard = state.public_connection.operation.lock() => guard,
            };
            if let Some(mut saved) = state.public_connection.registration() {
                // A persisted JWT may be near expiry after a restart. Renew it
                // before handing the first credential to the signaling connection.
                if last_refresh.is_none_or(|last| last.elapsed() >= Duration::from_secs(15 * 60)) {
                    let payload = machine_payload(&state, saved.device_name.clone());
                    let renewal = async {
                        match saved.refresh_token.as_deref() {
                            Some(token) => match mrd_device_registration::refresh_device(
                                &saved.api_url,
                                &payload,
                                token,
                            )
                            .await
                            {
                                Err(mrd_device_registration::DEVICE_CREDENTIAL_REJECTED) => {
                                    mrd_device_registration::register_device(
                                        &saved.api_url,
                                        &payload,
                                        None,
                                        Some(&saved.access_token),
                                    )
                                    .await
                                }
                                result => result,
                            },
                            None => {
                                mrd_device_registration::register_device(
                                    &saved.api_url,
                                    &payload,
                                    None,
                                    Some(&saved.access_token),
                                )
                                .await
                            }
                        }
                    };
                    let refreshed =
                        tokio::select! { _ = &mut stopping => break, result = renewal => result };
                    match refreshed {
                        Ok(value) if value.device_id == saved.device_id => {
                            match apply_registration(&state, &saved.api_url, value).await {
                                Ok(()) => {
                                    saved = state
                                        .public_connection
                                        .registration()
                                        .expect("successful protected save");
                                    last_refresh = Some(tokio::time::Instant::now());
                                }
                                Err(_) => {
                                    *state.public_connection.last_error.write().unwrap() =
                                        Some("public_credential_save_failed".into());
                                }
                            }
                        }
                        _ => {
                            *state.public_connection.last_error.write().unwrap() =
                                Some("public_credential_refresh_failed".into());
                        }
                    }
                }
                if let Some(credential) = &credential {
                    credential.replace(&saved.access_token);
                }
                if stack.is_none() {
                    if state.wan_session_backend().is_some() {
                        // Explicit legacy environment provisioners already own the WAN runtime.
                        *state.public_connection.last_error.write().unwrap() =
                            Some("public_runtime_config_conflict".into());
                        continue;
                    }
                    let shared = Arc::new(DeviceCredential::new(&saved.access_token));
                    match start_stack(&state, &saved, bootstrap, shared.clone()).await {
                        Ok(value) => {
                            stack = Some(value);
                            credential = Some(shared);
                        }
                        Err(error) => {
                            *state.public_connection.last_error.write().unwrap() =
                                Some("public_runtime_start_failed".into());
                            if error.is::<StackInstallFailed>() {
                                // This is an internal runtime failure, so no IPC
                                // response needs flushing before shutdown.
                                let mode = mrd_ipc::ShutdownMode::Graceful;
                                if state.shutdown.request(mode.clone()).is_ok() {
                                    state.shutdown.acknowledge(mode);
                                }
                                terminal_failure = Some(anyhow!(
                                    "public runtime installation failed; process restart required"
                                ));
                                break;
                            }
                        }
                    }
                }
            }
        }
        if let Some(stack) = stack {
            stack.shutdown().await;
        }
        match terminal_failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    });
    Ok(PublicConnectionTask {
        stop,
        join,
        temporary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn compatible_refresh_keeps_protected_capability_and_rejects_code_replacement() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let state = Arc::new(AppState::new());
        state
            .public_connection
            .configure_persistence(
                directory.path().to_path_buf(),
                Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([91; 32]).unwrap()),
                "compatible-machine",
            )
            .unwrap();
        let serial = state
            .public_connection
            .machine_serial
            .read()
            .unwrap()
            .clone();
        state
            .public_connection
            .save(Registration {
                device_id: "0123456789".into(),
                device_name: "Office".into(),
                access_token: "previous.access.token".into(),
                refresh_token: Some("protected.refresh.capability".into()),
                api_url: DEFAULT_PUBLIC_API_URL.into(),
                machine_serial: serial,
            })
            .unwrap();
        apply_registration(
            &state,
            DEFAULT_PUBLIC_API_URL,
            DeviceRegistrationResponse {
                device_id: "0123456789".into(),
                device_name: "Office".into(),
                access_token: "renewed.access.token".into(),
                refresh_token: None,
            },
        )
        .await
        .unwrap();
        let saved = state.public_connection.registration().unwrap();
        assert_eq!(
            saved.refresh_token.as_deref(),
            Some("protected.refresh.capability")
        );
        assert_eq!(saved.access_token, "renewed.access.token");
        assert!(apply_registration(
            &state,
            DEFAULT_PUBLIC_API_URL,
            DeviceRegistrationResponse {
                device_id: "1123456789".into(),
                device_name: "Wrong machine".into(),
                access_token: "wrong.access.token".into(),
                refresh_token: Some("wrong.refresh.capability".into()),
            }
        )
        .await
        .is_err());
        assert_eq!(
            state.public_connection.registration().unwrap().device_id,
            "0123456789"
        );
        let status = serde_json::to_string(
            &state
                .public_connection
                .snapshot(&signaling::SignalingRuntimeSnapshot::default()),
        )
        .unwrap();
        assert!(!status.contains("refresh"));
        assert!(!status.contains("token"));
    }

    fn fixture_registration() -> Registration {
        Registration {
            device_id: "0123456789".into(),
            device_name: "Office".into(),
            access_token: "synthetic-test-token".into(),
            refresh_token: None,
            api_url: "https://127.0.0.1:9/rdesk/api/v1".into(),
            machine_serial: "synthetic-test-machine".into(),
        }
    }

    fn fixture_bootstrap() -> Bootstrap {
        Bootstrap {
            signaling_url: "wss://127.0.0.1:9/signal".into(),
            signaling_server_device_id: "signal-server".into(),
            signaling_server_key_id: None,
            relay_directory_url: "https://127.0.0.1:9/rdesk/api/v1".into(),
            relay_directory_keys: [("test-key".into(), STANDARD.encode([17; 32]))].into(),
        }
    }

    #[tokio::test]
    async fn failed_wan_install_stops_and_joins_already_started_signaling() {
        let state = Arc::new(AppState::new());
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        state
            .public_connection
            .configure_persistence(
                directory.path().to_path_buf(),
                Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([72; 32]).unwrap()),
                state.device_identities.machine_key_id().unwrap(),
            )
            .unwrap();
        let saved = fixture_registration();
        // Missing local registration deliberately fails the final WAN install,
        // after signaling and the relay responder have been started.
        let error = start_stack(
            &state,
            &saved,
            fixture_bootstrap(),
            Arc::new(DeviceCredential::new(&saved.access_token)),
        )
        .await
        .err()
        .expect("WAN install must fail without local identity");
        assert!(error.is::<StackInstallFailed>());
        assert!(state.wan_session_backend().is_some());
        assert_eq!(
            state.signaling_status.snapshot().state,
            signaling::SignalingConnectionState::Stopped
        );
    }

    #[tokio::test]
    async fn invalid_configuration_does_not_bind_any_process_owner() {
        let state = Arc::new(AppState::new());
        let saved = fixture_registration();
        let mut bootstrap = fixture_bootstrap();
        bootstrap
            .relay_directory_keys
            .insert("test-key".into(), "invalid base64".into());
        assert!(start_stack(
            &state,
            &saved,
            bootstrap,
            Arc::new(DeviceCredential::new(&saved.access_token))
        )
        .await
        .is_err());
        assert!(state.wan_session_backend().is_none());
        assert!(state.relay_directory_client().is_none());
        assert!(state.relay_failover_coordinator().is_none());
        assert_eq!(
            state.signaling_status.snapshot().state,
            signaling::SignalingConnectionState::Disabled
        );
    }
    #[test]
    fn public_urls_reject_unsafe_origins_and_secret_urls() {
        for invalid in [
            "http://example.org/api",
            "https://secret@example.org/api",
            "https://example.org/api?token=secret",
            "https://example.org/api#secret",
        ] {
            assert!(validate_api_url(invalid).is_err());
        }
        assert!(validate_api_url(DEFAULT_PUBLIC_API_URL).is_ok());
    }
    #[test]
    fn refreshed_shared_token_is_available_to_existing_clients() {
        let token = Arc::new(DeviceCredential::new("first"));
        let client_token = token.clone();
        token.replace("second");
        assert_eq!(client_token.snapshot().as_str(), "second");
    }

    #[test]
    fn configured_signing_counter_is_shared_and_separate_from_device_credentials() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let state = PublicConnectionState::default();
        assert!(state.signaling_counter().unwrap().is_none());
        state
            .configure_persistence(
                directory.path().to_path_buf(),
                Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([73; 32]).unwrap()),
                "machine-one",
            )
            .unwrap();
        let first = state.signaling_counter().unwrap().unwrap();
        let second = state.signaling_counter().unwrap().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!directory.path().join("public-device-v1.protected").exists());
        drop(first);
        drop(second);
        drop(state);
        let restarted = PublicConnectionState::default();
        restarted
            .configure_persistence(
                directory.path().to_path_buf(),
                Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([73; 32]).unwrap()),
                "machine-one",
            )
            .unwrap();
        assert!(restarted.signaling_counter().unwrap().is_some());
    }

    #[test]
    fn restart_preserves_identity_while_credentials_stay_encrypted_and_machine_bound() {
        let directory =
            std::env::temp_dir().join(format!("mrd-public-store-{}", uuid_suffix().unwrap()));
        std::fs::create_dir(&directory).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let protector: Arc<dyn SecretProtector> =
            Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([71; 32]).unwrap());
        let first = PublicConnectionState::default();
        first
            .configure_persistence(directory.clone(), protector.clone(), "machine-one")
            .unwrap();
        first
            .save(Registration {
                device_id: "0123456789".into(),
                device_name: "Office".into(),
                access_token: "highly-sensitive-device-token".into(),
                refresh_token: Some("highly-sensitive-refresh-token".into()),
                api_url: DEFAULT_PUBLIC_API_URL.into(),
                machine_serial: "test-stable-machine".into(),
            })
            .unwrap();
        let path = directory.join("public-device-v1.protected");
        let encrypted = std::fs::read(&path).unwrap();
        assert!(!encrypted
            .windows(b"highly-sensitive".len())
            .any(|window| window == b"highly-sensitive"));
        let restarted = PublicConnectionState::default();
        restarted
            .configure_persistence(directory.clone(), protector.clone(), "machine-one")
            .unwrap();
        let status = restarted.snapshot(&signaling::SignalingRuntimeSnapshot::default());
        assert_eq!(status.device_id.as_deref(), Some("0123456789"));
        assert_eq!(
            restarted.registration().unwrap().refresh_token.as_deref(),
            Some("highly-sensitive-refresh-token")
        );
        assert_eq!(status.signaling_state, "disabled");
        assert!(!serde_json::to_string(&status)
            .unwrap()
            .contains("highly-sensitive"));
        assert!(PublicConnectionState::default()
            .configure_persistence(directory.clone(), protector.clone(), "different-machine")
            .is_err());
        let mut corrupted = encrypted;
        *corrupted.last_mut().unwrap() ^= 1;
        std::fs::write(&path, corrupted).unwrap();
        assert!(PublicConnectionState::default()
            .configure_persistence(directory.clone(), protector, "machine-one")
            .is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
