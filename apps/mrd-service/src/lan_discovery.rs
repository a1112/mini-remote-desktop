use crate::app_state::AppState;
use anyhow::{Context, Result};
use mrd_application::ports::SessionSnapshot;
use mrd_ipc::{LanDiscoverySnapshot, LanPeerInfo};
use mrd_proto::{DeviceId, SessionId};
use mrd_transport_quic_quinn::{
    fragment_access_unit, QuicAuReassembler, QuicAuReassemblerConfig, QuinnDatagramEndpoint,
    QuinnServerBootstrap, QuinnServerListener, QUIC_AU_FRAGMENT_HEADER_LEN,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, Notify};
use tokio::time::{interval, timeout};

const DEFAULT_DISCOVERY_PORT: u16 = 21116;
const PROTOCOL_VERSION: u32 = 1;
const ANNOUNCE_INTERVAL_SECS: u64 = 3;
const PEER_TTL_SECS: u64 = 12;
const DISCOVERY_MAGIC: &str = "mrd-lan-discovery-v1";
const DISCOVERY_APP_ID: &str = "rdesk";
const DISCOVERY_PACKET_BUFFER_BYTES: usize = 65_535;
const LAN_MEDIA_FRAME_INTERVAL_MS: u64 = 16;
const LAN_MEDIA_MAX_FRAMES: u64 = 3_600;
const LAN_MEDIA_PAYLOAD_BYTES: usize = 4_096;
const LAN_QUIC_FALLBACK_DATAGRAM_BYTES: usize = 1_200;
const LAN_QUIC_MEDIA_TRANSPORT: &str = "quic_datagram";

#[derive(Debug, Clone)]
pub struct LanDiscoveryConfig {
    pub enabled: bool,
    pub discovery_port: u16,
    pub announce_interval: Duration,
    pub peer_ttl: Duration,
}

impl Default for LanDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            discovery_port: DEFAULT_DISCOVERY_PORT,
            announce_interval: Duration::from_secs(ANNOUNCE_INTERVAL_SECS),
            peer_ttl: Duration::from_secs(PEER_TTL_SECS),
        }
    }
}

#[derive(Debug)]
pub struct LanDiscoveryState {
    config: LanDiscoveryConfig,
    instance_id: String,
    running: AtomicBool,
    last_probe_ms: AtomicU64,
    peers: Mutex<HashMap<String, StoredLanPeer>>,
    probe_requested: Notify,
    peer_changed: Notify,
}

impl LanDiscoveryState {
    pub fn new(config: LanDiscoveryConfig) -> Self {
        Self {
            config,
            instance_id: new_instance_id(),
            running: AtomicBool::new(false),
            last_probe_ms: AtomicU64::new(0),
            peers: Mutex::new(HashMap::new()),
            probe_requested: Notify::new(),
            peer_changed: Notify::new(),
        }
    }

    pub fn discovery_port(&self) -> u16 {
        self.config.discovery_port
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn request_probe(&self) {
        self.probe_requested.notify_one();
    }

    pub async fn request_probe_and_wait(&self, wait: Duration) -> LanDiscoverySnapshot {
        let notified = self.peer_changed.notified();
        self.request_probe();
        let _ = timeout(wait, notified).await;
        self.snapshot().await
    }

    async fn upsert_peer(&self, announcement: LanAnnouncement, addr: SocketAddr) {
        if announcement.instance_id == self.instance_id {
            return;
        }

        let peer = StoredLanPeer {
            device_id: announcement.device_id,
            device_name: announcement.device_name,
            device_type: announcement.device_type,
            ip: addr.ip(),
            discovery_port: announcement.discovery_port,
            transports: announcement.transports,
            protocol_version: announcement.protocol_version,
            last_seen_ms: now_ms(),
        };

        self.peers.lock().await.insert(peer.device_id.clone(), peer);
        self.peer_changed.notify_one();
    }

    async fn prune_stale_peers(&self) {
        let ttl_ms = self.config.peer_ttl.as_millis() as u64;
        let now = now_ms();
        self.peers
            .lock()
            .await
            .retain(|_, peer| now.saturating_sub(peer.last_seen_ms) <= ttl_ms);
    }

    pub async fn snapshot(&self) -> LanDiscoverySnapshot {
        self.prune_stale_peers().await;
        let now = now_ms();
        let peers = self
            .peers
            .lock()
            .await
            .values()
            .map(|peer| {
                let p2p_control_addr = SocketAddr::new(peer.ip, peer.discovery_port).to_string();
                LanPeerInfo {
                    device_id: DeviceId(peer.device_id.clone()),
                    device_name: peer.device_name.clone(),
                    device_type: peer.device_type.clone(),
                    ip: peer.ip.to_string(),
                    discovery_port: peer.discovery_port,
                    p2p_control_addr,
                    transports: peer.transports.clone(),
                    protocol_version: peer.protocol_version,
                    age_ms: now.saturating_sub(peer.last_seen_ms),
                    p2p_available: true,
                }
            })
            .collect();

        let last_probe = self.last_probe_ms.load(Ordering::Relaxed);
        LanDiscoverySnapshot {
            enabled: self.config.enabled,
            running: self.running.load(Ordering::Relaxed),
            discovery_port: self.config.discovery_port,
            instance_id: self.instance_id.clone(),
            last_probe_ms: if last_probe == 0 {
                None
            } else {
                Some(last_probe)
            },
            peers,
        }
    }

    pub async fn peer_control_addr(&self, device_id: &DeviceId) -> Option<SocketAddr> {
        self.prune_stale_peers().await;
        self.peers
            .lock()
            .await
            .get(&device_id.0)
            .map(|peer| SocketAddr::new(peer.ip, peer.discovery_port))
    }

    pub async fn peer_transports(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.prune_stale_peers().await;
        self.peers
            .lock()
            .await
            .get(&device_id.0)
            .map(|peer| peer.transports.clone())
    }
}

impl Default for LanDiscoveryState {
    fn default() -> Self {
        Self::new(LanDiscoveryConfig::default())
    }
}

#[derive(Debug, Clone)]
struct StoredLanPeer {
    device_id: String,
    device_name: String,
    device_type: String,
    ip: IpAddr,
    discovery_port: u16,
    transports: Vec<String>,
    protocol_version: u32,
    last_seen_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LanDiscoveryPacket {
    Probe {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        device_id: Option<String>,
        timestamp_ms: u64,
    },
    Announce(LanAnnouncement),
    RemoteSessionRequest {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        source_device_name: String,
        transport_kind: String,
        #[serde(default)]
        source_discovery_port: Option<u16>,
        timestamp_ms: u64,
    },
    RemoteSessionAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media: Option<LanMediaBootstrap>,
        timestamp_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanAnnouncement {
    magic: String,
    #[serde(default = "default_app_id")]
    app_id: String,
    instance_id: String,
    device_id: String,
    device_name: String,
    device_type: String,
    protocol_version: u32,
    discovery_port: u16,
    transports: Vec<String>,
    timestamp_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanMediaBootstrap {
    transport_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quic: Option<LanQuicBootstrap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanQuicBootstrap {
    listen_addr: String,
    server_name: String,
    cert_der: Vec<u8>,
}

struct LanRemoteAcceptResult {
    accepted: bool,
    message: Option<String>,
    media: Option<LanMediaBootstrap>,
}

impl LanRemoteAcceptResult {
    fn rejected(message: impl Into<String>) -> Self {
        Self {
            accepted: false,
            message: Some(message.into()),
            media: None,
        }
    }
}

pub async fn send_probe(
    socket: &UdpSocket,
    discovery_port: u16,
    state: &LanDiscoveryState,
) -> Result<()> {
    let packet = LanDiscoveryPacket::Probe {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: state.instance_id.clone(),
        device_id: None,
        timestamp_ms: now_ms(),
    };
    send_packet(
        socket,
        &packet,
        SocketAddr::from(([255, 255, 255, 255], discovery_port)),
    )
    .await?;
    state.last_probe_ms.store(now_ms(), Ordering::Relaxed);
    Ok(())
}

pub async fn start_lan_discovery(app_state: Arc<AppState>) -> Result<()> {
    if !app_state.lan_discovery.config.enabled {
        return Ok(());
    }

    let port = app_state.lan_discovery.discovery_port();
    let socket = Arc::new(
        UdpSocket::bind(("0.0.0.0", port))
            .await
            .with_context(|| format!("failed to bind LAN discovery UDP port {port}"))?,
    );
    socket
        .set_broadcast(true)
        .context("failed to enable LAN discovery UDP broadcast")?;

    app_state
        .lan_discovery
        .running
        .store(true, Ordering::Relaxed);

    let receive_socket = socket.clone();
    let receive_state = app_state.clone();
    tokio::spawn(async move {
        receive_loop(receive_socket, receive_state).await;
    });

    let announce_socket = socket.clone();
    let announce_state = app_state.clone();
    tokio::spawn(async move {
        announce_loop(announce_socket, announce_state).await;
    });

    send_probe(&socket, port, &app_state.lan_discovery).await?;
    Ok(())
}

pub async fn request_lan_remote_session(
    app_state: &Arc<AppState>,
    target_device_id: &DeviceId,
    session_id: &SessionId,
    transport_kind: &str,
) -> Result<()> {
    let target = app_state
        .lan_discovery
        .peer_control_addr(target_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", target_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(target_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", target_device_id.0))?;
    ensure_peer_supports_requested_media(target_device_id, transport_kind, &peer_transports)?;

    let (source_device_id, source_device_name) = {
        let devices = app_state.devices.lock().await;
        devices
            .get_local_device()
            .map(|(id, name)| (id.0.clone(), name.clone()))
            .context("local device is not registered")?
    };

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN remote request UDP socket")?;
    let packet = LanDiscoveryPacket::RemoteSessionRequest {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        source_device_name,
        transport_kind: transport_kind.to_string(),
        source_discovery_port: Some(app_state.lan_discovery.discovery_port()),
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, ack_addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
        .await
        .context("LAN remote request timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::RemoteSessionAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            media,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                start_lan_media_receiver(
                    app_state.clone(),
                    session_id.clone(),
                    transport_kind,
                    media,
                    ack_addr.ip(),
                )
                .await?;
                Ok(())
            } else {
                anyhow::bail!(
                    "LAN peer rejected remote session: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN remote session response"),
    }
}

async fn announce_loop(socket: Arc<UdpSocket>, app_state: Arc<AppState>) {
    let mut ticker = interval(app_state.lan_discovery.config.announce_interval);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Some(announcement) = build_announcement(&app_state).await {
                    let packet = LanDiscoveryPacket::Announce(announcement);
                    let target = SocketAddr::from(([255, 255, 255, 255], app_state.lan_discovery.discovery_port()));
                    if let Err(error) = send_packet(&socket, &packet, target).await {
                        tracing::warn!(%error, "failed to send LAN discovery announce");
                    } else {
                        app_state
                            .lan_discovery
                            .last_probe_ms
                            .store(now_ms(), Ordering::Relaxed);
                    }
                }
                app_state.lan_discovery.prune_stale_peers().await;
            }
            _ = app_state.lan_discovery.probe_requested.notified() => {
                if let Err(error) = send_probe(&socket, app_state.lan_discovery.discovery_port(), &app_state.lan_discovery).await {
                    tracing::warn!(%error, "failed to send LAN discovery probe");
                }
            }
        }
    }
}

async fn receive_loop(socket: Arc<UdpSocket>, app_state: Arc<AppState>) {
    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    loop {
        match socket.recv_from(&mut buffer).await {
            Ok((len, addr)) => {
                if let Err(error) = handle_packet(&socket, &app_state, &buffer[..len], addr).await {
                    tracing::debug!(%error, %addr, "ignored LAN discovery packet");
                }
            }
            Err(error) => {
                tracing::warn!(%error, "LAN discovery UDP receive failed");
            }
        }
    }
}

async fn handle_packet(
    socket: &UdpSocket,
    app_state: &Arc<AppState>,
    bytes: &[u8],
    addr: SocketAddr,
) -> Result<()> {
    let packet: LanDiscoveryPacket = serde_json::from_slice(bytes)?;
    match packet {
        LanDiscoveryPacket::Probe {
            magic,
            app_id,
            instance_id,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }
            if let Some(announcement) = build_announcement(app_state).await {
                send_packet(socket, &LanDiscoveryPacket::Announce(announcement), addr).await?;
            }
        }
        LanDiscoveryPacket::Announce(announcement) => {
            if is_valid_discovery_packet(&announcement.magic, &announcement.app_id) {
                app_state
                    .lan_discovery
                    .upsert_peer(announcement, addr)
                    .await;
            }
        }
        LanDiscoveryPacket::RemoteSessionRequest {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            transport_kind,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let accept_result = accept_lan_remote_session(
                app_state,
                SessionId(session_id.clone()),
                DeviceId(source_device_id),
                transport_kind,
            )
            .await;

            let ack = LanDiscoveryPacket::RemoteSessionAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id: session_id.clone(),
                accepted: accept_result.accepted,
                message: accept_result.message,
                media: accept_result.media,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::RemoteSessionAck { .. } => {}
    }

    Ok(())
}

async fn accept_lan_remote_session(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    source_device_id: DeviceId,
    transport_kind: String,
) -> LanRemoteAcceptResult {
    let is_registered = {
        let devices = app_state.devices.lock().await;
        devices.is_registered()
    };
    if !is_registered {
        return LanRemoteAcceptResult::rejected("local device is not registered");
    }

    let transport = normalize_transport_kind(&transport_kind);
    if transport == "webrtc" {
        return LanRemoteAcceptResult::rejected(
            "LAN WebRTC media path is not implemented in mrd-service yet",
        );
    }
    if transport != "quic" {
        return LanRemoteAcceptResult::rejected(format!(
            "unsupported LAN media transport: {transport}"
        ));
    }

    let (listener, bootstrap) = match QuinnServerListener::bind("0.0.0.0:0").await {
        Ok(value) => value,
        Err(error) => {
            return LanRemoteAcceptResult::rejected(format!(
                "failed to start LAN QUIC media listener: {error}"
            ));
        }
    };

    let local_media = LanMediaBootstrap {
        transport_kind: "quic".to_string(),
        quic: Some(LanQuicBootstrap {
            listen_addr: bootstrap.listen_addr.to_string(),
            server_name: bootstrap.server_name.clone(),
            cert_der: bootstrap.cert_der.clone(),
        }),
    };
    spawn_quic_media_sender(session_id.clone(), listener);

    let local_listen_addr = bootstrap.listen_addr.to_string();
    let local_server_name = bootstrap.server_name.clone();
    {
        let mut sessions = app_state.sessions.lock().await;
        sessions.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id,
                transport,
                source_device_id: Some(source_device_id),
                target_device_id: None,
                local_listen_addr: Some(local_listen_addr),
                local_server_name: Some(local_server_name),
                local_cert_der_b64: None,
                remote_listen_addr: None,
                remote_server_name: None,
                remote_cert_der_b64: None,
                lifecycle_state: "listening".to_string(),
                last_error: None,
                sender_active: true,
                receiver_active: false,
            },
        );
    }

    LanRemoteAcceptResult {
        accepted: true,
        message: Some("accepted".to_string()),
        media: Some(local_media),
    }
}

async fn build_announcement(app_state: &Arc<AppState>) -> Option<LanAnnouncement> {
    let (device_id, device_name) = {
        let devices = app_state.devices.lock().await;
        devices
            .get_local_device()
            .map(|(id, name)| (id.0.clone(), name.clone()))
    }?;

    Some(LanAnnouncement {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        device_id,
        device_name,
        device_type: "rdesk".to_string(),
        protocol_version: PROTOCOL_VERSION,
        discovery_port: app_state.lan_discovery.discovery_port(),
        transports: vec!["quic".to_string(), LAN_QUIC_MEDIA_TRANSPORT.to_string()],
        timestamp_ms: now_ms(),
    })
}

async fn send_packet(
    socket: &UdpSocket,
    packet: &LanDiscoveryPacket,
    target: SocketAddr,
) -> Result<()> {
    let bytes = serde_json::to_vec(packet)?;
    socket.send_to(&bytes, target).await?;
    Ok(())
}

async fn start_lan_media_receiver(
    app_state: Arc<AppState>,
    session_id: SessionId,
    requested_transport: &str,
    media: Option<LanMediaBootstrap>,
    peer_ip: IpAddr,
) -> Result<()> {
    let requested_transport = normalize_transport_kind(requested_transport);
    if requested_transport == "webrtc" {
        anyhow::bail!("LAN WebRTC media path is not implemented in mrd-service yet");
    }
    if requested_transport != "quic" {
        anyhow::bail!("unsupported LAN media transport: {requested_transport}");
    }

    let media = media.context("LAN peer accepted session without media bootstrap")?;
    if normalize_transport_kind(&media.transport_kind) != "quic" {
        anyhow::bail!(
            "LAN peer returned unexpected media transport: {}",
            media.transport_kind
        );
    }
    let quic = media
        .quic
        .context("LAN peer accepted QUIC session without QUIC bootstrap")?;
    let bootstrap = quic_bootstrap_for_peer(quic.clone(), peer_ip)?;
    let endpoint = QuinnDatagramEndpoint::connect_client("0.0.0.0:0", &bootstrap)
        .await
        .context("failed to connect LAN QUIC media receiver")?;

    {
        let mut sessions = app_state.sessions.lock().await;
        if let Some(snapshot) = sessions.get(&session_id).cloned() {
            sessions.insert(
                session_id.clone(),
                SessionSnapshot {
                    remote_listen_addr: Some(bootstrap.listen_addr.to_string()),
                    remote_server_name: Some(bootstrap.server_name.clone()),
                    remote_cert_der_b64: None,
                    ..snapshot
                },
            );
        }
    }

    spawn_quic_media_receiver(app_state, session_id, endpoint);
    Ok(())
}

fn quic_bootstrap_for_peer(
    quic: LanQuicBootstrap,
    peer_ip: IpAddr,
) -> Result<QuinnServerBootstrap> {
    let listen_addr = quic
        .listen_addr
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid LAN QUIC listen addr: {}", quic.listen_addr))?;
    Ok(QuinnServerBootstrap {
        transport: "quic_quinn",
        listen_addr: SocketAddr::new(peer_ip, listen_addr.port()),
        server_name: quic.server_name,
        cert_der: quic.cert_der,
    })
}

fn ensure_peer_supports_requested_media(
    target_device_id: &DeviceId,
    transport_kind: &str,
    peer_transports: &[String],
) -> Result<()> {
    let transport = normalize_transport_kind(transport_kind);
    if transport == "quic"
        && !peer_transports
            .iter()
            .any(|peer_transport| peer_transport.eq_ignore_ascii_case(LAN_QUIC_MEDIA_TRANSPORT))
    {
        anyhow::bail!(
            "LAN peer does not advertise {LAN_QUIC_MEDIA_TRANSPORT} media capability: {} supports {}",
            target_device_id.0,
            format_peer_transports(peer_transports)
        );
    }
    Ok(())
}

fn format_peer_transports(peer_transports: &[String]) -> String {
    if peer_transports.is_empty() {
        "none".to_string()
    } else {
        peer_transports.join(", ")
    }
}

fn spawn_quic_media_sender(session_id: SessionId, listener: QuinnServerListener) {
    tokio::spawn(async move {
        let local_addr = listener.local_addr();
        let result = async move {
            let endpoint = listener
                .accept()
                .await
                .context("LAN QUIC media listener failed to accept receiver")?;
            send_quic_media_loop(endpoint, session_id).await
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, %local_addr, "LAN QUIC media sender stopped");
        }
    });
}

async fn send_quic_media_loop(
    endpoint: QuinnDatagramEndpoint,
    session_id: SessionId,
) -> Result<()> {
    let mut ticker = interval(Duration::from_millis(LAN_MEDIA_FRAME_INTERVAL_MS));
    let max_datagram_size = endpoint
        .max_datagram_size()
        .unwrap_or(LAN_QUIC_FALLBACK_DATAGRAM_BYTES)
        .min(LAN_QUIC_FALLBACK_DATAGRAM_BYTES)
        .max(QUIC_AU_FRAGMENT_HEADER_LEN + 1);

    for frame_id in 1..=LAN_MEDIA_MAX_FRAMES {
        ticker.tick().await;
        let mut payload = vec![0_u8; LAN_MEDIA_PAYLOAD_BYTES];
        payload[..8].copy_from_slice(&frame_id.to_le_bytes());
        let fragments = fragment_access_unit(
            frame_id as u32,
            now_ms().saturating_mul(1_000),
            frame_id == 1,
            &payload,
            max_datagram_size,
        )
        .context("failed to fragment LAN QUIC media frame")?;

        for fragment in fragments {
            endpoint
                .send_datagram(fragment)
                .with_context(|| format!("failed to send LAN QUIC media frame {}", frame_id))?;
        }
    }

    tracing::debug!(session_id = %session_id.0, "LAN QUIC media sender completed");
    Ok(())
}

fn spawn_quic_media_receiver(
    app_state: Arc<AppState>,
    session_id: SessionId,
    endpoint: QuinnDatagramEndpoint,
) {
    tokio::spawn(async move {
        if let Err(error) = receive_quic_media_loop(app_state, session_id.clone(), endpoint).await {
            tracing::warn!(%error, session_id = %session_id.0, "LAN QUIC media receiver stopped");
        }
    });
}

async fn receive_quic_media_loop(
    app_state: Arc<AppState>,
    session_id: SessionId,
    endpoint: QuinnDatagramEndpoint,
) -> Result<()> {
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default());
    loop {
        let datagram = endpoint.read_datagram().await?;
        if let Some(frame) = reassembler
            .push_datagram(&datagram)
            .context("failed to reassemble LAN QUIC media frame")?
        {
            app_state.probes.lock().await.record_probe_frame(
                &session_id,
                frame.payload.len() as u64,
                now_ms(),
            );
        }
    }
}

fn normalize_transport_kind(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "quic_quinn" {
        "quic".to_string()
    } else {
        normalized
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn new_instance_id() -> String {
    format!("mrd-{}-{}", std::process::id(), now_ms())
}

fn default_app_id() -> String {
    DISCOVERY_APP_ID.to_string()
}

fn is_valid_discovery_packet(magic: &str, app_id: &str) -> bool {
    magic == DISCOVERY_MAGIC && app_id.eq_ignore_ascii_case(DISCOVERY_APP_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snapshot_exposes_recent_peer() {
        let state = LanDiscoveryState::default();
        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "remote-instance".to_string(),
                    device_id: "remote-device".to_string(),
                    device_name: "Remote Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: 21116,
                    transports: vec!["webrtc".to_string()],
                    timestamp_ms: now_ms(),
                },
                "192.168.1.50:21116".parse().unwrap(),
            )
            .await;

        let snapshot = state.snapshot().await;
        assert_eq!(snapshot.peers.len(), 1);
        assert_eq!(snapshot.peers[0].device_id.0, "remote-device");
        assert_eq!(snapshot.peers[0].p2p_control_addr, "192.168.1.50:21116");
        assert!(snapshot.peers[0].p2p_available);
    }

    #[tokio::test]
    async fn peer_control_addr_returns_discovered_endpoint() {
        let state = LanDiscoveryState::default();
        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "remote-instance".to_string(),
                    device_id: "remote-device".to_string(),
                    device_name: "Remote Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: 21117,
                    transports: vec!["webrtc".to_string()],
                    timestamp_ms: now_ms(),
                },
                "192.168.1.50:21116".parse().unwrap(),
            )
            .await;

        let addr = state
            .peer_control_addr(&DeviceId("remote-device".to_string()))
            .await
            .expect("peer addr");

        assert_eq!(addr.to_string(), "192.168.1.50:21117");
    }

    #[tokio::test]
    async fn remote_session_request_auto_accepts_session() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );

        let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ack_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let request = LanDiscoveryPacket::RemoteSessionRequest {
            magic: DISCOVERY_MAGIC.to_string(),
            app_id: DISCOVERY_APP_ID.to_string(),
            instance_id: "controller-instance".to_string(),
            session_id: "session-1".to_string(),
            source_device_id: "controller-device".to_string(),
            source_device_name: "Controller".to_string(),
            transport_kind: "quic".to_string(),
            source_discovery_port: Some(21116),
            timestamp_ms: now_ms(),
        };
        let bytes = serde_json::to_vec(&request).unwrap();

        handle_packet(
            &service_socket,
            &app_state,
            &bytes,
            ack_socket.local_addr().unwrap(),
        )
        .await
        .unwrap();

        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, _) = timeout(Duration::from_secs(1), ack_socket.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len]).unwrap();
        match ack {
            LanDiscoveryPacket::RemoteSessionAck {
                session_id,
                accepted,
                media,
                ..
            } => {
                assert_eq!(session_id, "session-1");
                assert!(accepted);
                let media = media.expect("QUIC media bootstrap");
                assert_eq!(media.transport_kind, "quic");
                let quic = media.quic.expect("QUIC bootstrap details");
                assert!(quic.listen_addr.ends_with(":0") == false);
                assert!(!quic.server_name.is_empty());
                assert!(!quic.cert_der.is_empty());
            }
            _ => panic!("expected remote session ack"),
        }

        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions
            .get(&SessionId("session-1".to_string()))
            .expect("accepted session");
        assert_eq!(
            snapshot.source_device_id,
            Some(DeviceId("controller-device".to_string()))
        );
        assert_eq!(snapshot.transport, "quic");
        assert_eq!(snapshot.lifecycle_state, "listening");
        assert!(snapshot.sender_active);
        assert!(snapshot.local_listen_addr.is_some());
    }

    #[tokio::test]
    async fn remote_session_request_rejects_webrtc_until_media_path_exists() {
        let app_state = Arc::new(AppState::new());
        app_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );

        let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ack_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let request = LanDiscoveryPacket::RemoteSessionRequest {
            magic: DISCOVERY_MAGIC.to_string(),
            app_id: DISCOVERY_APP_ID.to_string(),
            instance_id: "controller-instance".to_string(),
            session_id: "session-1".to_string(),
            source_device_id: "controller-device".to_string(),
            source_device_name: "Controller".to_string(),
            transport_kind: "webrtc".to_string(),
            source_discovery_port: None,
            timestamp_ms: now_ms(),
        };
        let bytes = serde_json::to_vec(&request).unwrap();

        handle_packet(
            &service_socket,
            &app_state,
            &bytes,
            ack_socket.local_addr().unwrap(),
        )
        .await
        .unwrap();

        let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
        let (len, _) = timeout(Duration::from_secs(1), ack_socket.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len]).unwrap();
        match ack {
            LanDiscoveryPacket::RemoteSessionAck {
                accepted, message, ..
            } => {
                assert!(!accepted);
                assert!(message
                    .expect("reject message")
                    .contains("WebRTC media path is not implemented"));
            }
            _ => panic!("expected remote session ack"),
        }
    }

    #[tokio::test]
    async fn request_lan_remote_session_records_quic_datagram_frames() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );
        tokio::time::sleep(Duration::from_millis(1)).await;

        let target_state = Arc::new(AppState::new());
        target_state.devices.lock().await.register(
            DeviceId("target-device".to_string()),
            "Target Device".to_string(),
        );

        let service_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let service_addr = service_socket.local_addr().unwrap();
        let handler_socket = service_socket.clone();
        let handler_state = target_state.clone();
        let handler = tokio::spawn(async move {
            let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
            let (len, addr) = handler_socket.recv_from(&mut buffer).await.unwrap();
            handle_packet(&handler_socket, &handler_state, &buffer[..len], addr)
                .await
                .unwrap();
        });

        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: service_addr.port(),
                    transports: vec!["quic".to_string(), LAN_QUIC_MEDIA_TRANSPORT.to_string()],
                    timestamp_ms: now_ms(),
                },
                service_addr,
            )
            .await;

        let session_id = SessionId("session-quic-media".to_string());
        request_lan_remote_session(
            &controller_state,
            &DeviceId("target-device".to_string()),
            &session_id,
            "quic",
        )
        .await
        .unwrap();
        handler.await.unwrap();

        let mut snapshot = controller_state.probes.lock().await.snapshot(&session_id);
        for _ in 0..40 {
            if snapshot.frames_received > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            snapshot = controller_state.probes.lock().await.snapshot(&session_id);
        }

        assert!(snapshot.frames_received > 0);
        assert!(snapshot.frames_decoded > 0);
    }

    #[tokio::test]
    async fn request_lan_remote_session_rejects_legacy_quic_peer_without_media_capability() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );

        let peer_addr: SocketAddr = "127.0.0.1:32216".parse().unwrap();
        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "legacy-target-instance".to_string(),
                    device_id: "legacy-target-device".to_string(),
                    device_name: "Legacy Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: peer_addr.port(),
                    transports: vec!["quic".to_string()],
                    timestamp_ms: now_ms(),
                },
                peer_addr,
            )
            .await;

        let error = request_lan_remote_session(
            &controller_state,
            &DeviceId("legacy-target-device".to_string()),
            &SessionId("session-legacy-peer".to_string()),
            "quic",
        )
        .await
        .expect_err("legacy QUIC peer should fail before session request");

        assert!(error.to_string().contains("quic_datagram"));
    }

    #[tokio::test]
    async fn snapshot_ignores_own_instance() {
        let state = LanDiscoveryState::default();
        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: state.instance_id().to_string(),
                    device_id: "self-device".to_string(),
                    device_name: "Self".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: 21116,
                    transports: vec!["webrtc".to_string()],
                    timestamp_ms: now_ms(),
                },
                "127.0.0.1:21116".parse().unwrap(),
            )
            .await;

        assert!(state.snapshot().await.peers.is_empty());
    }

    #[tokio::test]
    async fn request_probe_and_wait_returns_after_peer_update() {
        let state = Arc::new(LanDiscoveryState::default());
        let waiting_state = state.clone();
        let waiter = tokio::spawn(async move {
            waiting_state
                .request_probe_and_wait(Duration::from_secs(1))
                .await
        });

        state
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "remote-instance".to_string(),
                    device_id: "remote-device".to_string(),
                    device_name: "Remote Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: 21116,
                    transports: vec!["webrtc".to_string(), "quic".to_string()],
                    timestamp_ms: now_ms(),
                },
                "192.168.1.50:21116".parse().unwrap(),
            )
            .await;

        let snapshot = waiter.await.unwrap();
        assert_eq!(snapshot.peers.len(), 1);
        assert_eq!(snapshot.peers[0].device_id.0, "remote-device");
    }

    #[test]
    fn discovery_packet_requires_rdesk_namespace() {
        assert!(is_valid_discovery_packet(DISCOVERY_MAGIC, DISCOVERY_APP_ID));
        assert!(!is_valid_discovery_packet(DISCOVERY_MAGIC, "rsharemouse"));
    }
}
