use crate::app_state::{AppState, MediaProbeFrameStats};
use anyhow::{Context, Result};
use mrd_application::ports::SessionSnapshot;
use mrd_ipc::{
    CaptureSource, CaptureSourceSelection, LanDiscoverySnapshot, LanPeerInfo, MediaProfile,
    MediaProfileNegotiation,
};
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
const DISCOVERY_SAFE_UDP_PAYLOAD_BYTES: usize = 60_000;
const LAN_MEDIA_TARGET_WIDTH: u32 = 2560;
const LAN_MEDIA_TARGET_HEIGHT: u32 = 1440;
const LAN_MEDIA_TARGET_FPS: u32 = 144;
const LAN_MEDIA_TARGET_BITRATE_MBPS: u32 = 64;
const LAN_QUIC_FALLBACK_DATAGRAM_BYTES: usize = 1_200;
const LAN_QUIC_MEDIA_TRANSPORT: &str = "quic_datagram";
const LAN_QUIC_MEDIA_PROFILE_TRANSPORT: &str = "quic_datagram_2k144";
const LAN_MEDIA_PROFILE_CONTROL_TRANSPORT: &str = "media_profile_control_v1";
const LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT: &str = "capture_source_control_v1";
const LAN_MEDIA_PROBE_MAGIC: &[u8; 8] = b"MRDMPF01";
const LAN_MEDIA_PROBE_HEADER_BYTES: usize = 56;
const LAN_MEDIA_PROBE_2K144_FORMAT: &str = "compressed_2k144_test_pattern";
const LAN_MEDIA_PROBE_DYNAMIC_FORMAT: &str = "compressed_h264_test_pattern";
const LAN_MEDIA_PROBE_FORMAT_CODE: u32 = 2;

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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_media_profile: Option<MediaProfile>,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_profile: Option<MediaProfileNegotiation>,
        timestamp_ms: u64,
    },
    MediaProfileUpdate {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        requested_media_profile: MediaProfile,
        timestamp_ms: u64,
    },
    MediaProfileUpdateAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_profile: Option<MediaProfileNegotiation>,
        timestamp_ms: u64,
    },
    CaptureSourcesRequest {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        include_previews: bool,
        limit: Option<u32>,
        timestamp_ms: u64,
    },
    CaptureSourcesAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        sources: Vec<CaptureSource>,
        timestamp_ms: u64,
    },
    CaptureSourceSelect {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        source_device_id: String,
        source_id: String,
        timestamp_ms: u64,
    },
    CaptureSourceSelectAck {
        magic: String,
        #[serde(default = "default_app_id")]
        app_id: String,
        instance_id: String,
        session_id: String,
        accepted: bool,
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<CaptureSourceSelection>,
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
    media_profile: Option<MediaProfileNegotiation>,
}

impl LanRemoteAcceptResult {
    fn rejected(message: impl Into<String>) -> Self {
        Self {
            accepted: false,
            message: Some(message.into()),
            media: None,
            media_profile: None,
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
    requested_profile: Option<MediaProfile>,
) -> Result<MediaProfileNegotiation> {
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
        requested_media_profile: requested_profile,
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
            media_profile,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                let negotiation = media_profile.unwrap_or_else(default_media_profile_negotiation);
                app_state
                    .media_profiles
                    .lock()
                    .await
                    .set(session_id.clone(), negotiation.clone());
                start_lan_media_receiver(
                    app_state.clone(),
                    session_id.clone(),
                    transport_kind,
                    media,
                    ack_addr.ip(),
                )
                .await?;
                Ok(negotiation)
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

pub async fn request_lan_media_profile_update(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    requested_profile: MediaProfile,
) -> Result<MediaProfileNegotiation> {
    validate_media_profile(&requested_profile)?;
    let peer_device_id = {
        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions
            .get(session_id)
            .with_context(|| format!("session not found: {}", session_id.0))?;
        snapshot
            .target_device_id
            .clone()
            .or_else(|| snapshot.source_device_id.clone())
            .with_context(|| format!("session has no remote peer: {}", session_id.0))?
    };
    let target = app_state
        .lan_discovery
        .peer_control_addr(&peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(&peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    ensure_peer_supports_requested_media(&peer_device_id, "quic", &peer_transports)?;

    let source_device_id = {
        let devices = app_state.devices.lock().await;
        devices
            .get_local_device()
            .map(|(id, _)| id.0.clone())
            .context("local device is not registered")?
    };

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN media profile update UDP socket")?;
    let packet = LanDiscoveryPacket::MediaProfileUpdate {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        requested_media_profile: requested_profile,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
        .await
        .context("LAN media profile update timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::MediaProfileUpdateAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            media_profile,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                let negotiation =
                    media_profile.context("LAN peer accepted profile update without result")?;
                app_state
                    .media_profiles
                    .lock()
                    .await
                    .set(session_id.clone(), negotiation.clone());
                Ok(negotiation)
            } else {
                anyhow::bail!(
                    "LAN peer rejected media profile update: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN media profile update response"),
    }
}

pub async fn request_lan_capture_sources(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    include_previews: bool,
    limit: Option<u32>,
) -> Result<Vec<CaptureSource>> {
    let peer_device_id = session_remote_peer(app_state, session_id).await?;
    let target =
        peer_control_addr_with_capture_source_capability(app_state, &peer_device_id).await?;
    let source_device_id = local_device_id(app_state).await?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN capture sources request UDP socket")?;
    let packet = LanDiscoveryPacket::CaptureSourcesRequest {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        include_previews,
        limit,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(3), socket.recv_from(&mut buffer))
        .await
        .context("LAN capture sources request timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::CaptureSourcesAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            sources,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                Ok(sources)
            } else {
                anyhow::bail!(
                    "LAN peer rejected capture source listing: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN capture sources response"),
    }
}

pub async fn request_lan_capture_source_select(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: String,
) -> Result<CaptureSourceSelection> {
    let peer_device_id = session_remote_peer(app_state, session_id).await?;
    let target =
        peer_control_addr_with_capture_source_capability(app_state, &peer_device_id).await?;
    let source_device_id = local_device_id(app_state).await?;

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN capture source select UDP socket")?;
    let packet = LanDiscoveryPacket::CaptureSourceSelect {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        source_id,
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; DISCOVERY_PACKET_BUFFER_BYTES];
    let (len, _) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
        .await
        .context("LAN capture source select timed out")??;
    let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
    match ack {
        LanDiscoveryPacket::CaptureSourceSelectAck {
            magic,
            app_id,
            session_id: ack_session_id,
            accepted,
            message,
            selection,
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
                let selection =
                    selection.context("LAN peer accepted capture source without selection")?;
                app_state
                    .capture_sources
                    .lock()
                    .await
                    .set(session_id.clone(), selection.clone());
                Ok(selection)
            } else {
                anyhow::bail!(
                    "LAN peer rejected capture source select: {}",
                    message.unwrap_or_else(|| "unknown reason".to_string())
                );
            }
        }
        _ => anyhow::bail!("unexpected LAN capture source select response"),
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
            requested_media_profile,
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
                requested_media_profile,
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
                media_profile: accept_result.media_profile,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::RemoteSessionAck { .. } => {}
        LanDiscoveryPacket::MediaProfileUpdate {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            requested_media_profile,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let session_id = SessionId(session_id);
            let update_result =
                accept_lan_media_profile_update(app_state, &session_id, requested_media_profile)
                    .await;
            let (accepted, message, media_profile) = match update_result {
                Ok(negotiation) => (true, Some("updated".to_string()), Some(negotiation)),
                Err(error) => (false, Some(error.to_string()), None),
            };
            tracing::info!(
                session_id = %session_id.0,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN media profile update"
            );
            let ack = LanDiscoveryPacket::MediaProfileUpdateAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id: session_id.0,
                accepted,
                message,
                media_profile,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::MediaProfileUpdateAck { .. } => {}
        LanDiscoveryPacket::CaptureSourcesRequest {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            include_previews,
            limit,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let sources_result = accept_lan_capture_sources_request(
                app_state,
                &SessionId(session_id.clone()),
                include_previews,
                limit,
            )
            .await;
            let (accepted, message, sources) = match sources_result {
                Ok(sources) => (true, Some("listed".to_string()), sources),
                Err(error) => (false, Some(error.to_string()), Vec::new()),
            };
            tracing::info!(
                session_id = %session_id,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN capture sources request"
            );
            let ack = fit_capture_sources_ack_packet(
                app_state.lan_discovery.instance_id.clone(),
                session_id,
                accepted,
                message,
                sources,
            );
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::CaptureSourcesAck { .. } => {}
        LanDiscoveryPacket::CaptureSourceSelect {
            magic,
            app_id,
            instance_id,
            session_id,
            source_device_id,
            source_id,
            ..
        } => {
            if !is_valid_discovery_packet(&magic, &app_id)
                || instance_id == app_state.lan_discovery.instance_id()
            {
                return Ok(());
            }

            let session_id = SessionId(session_id);
            let select_result =
                accept_lan_capture_source_select(app_state, &session_id, &source_id).await;
            let (accepted, message, selection) = match select_result {
                Ok(selection) => (true, Some("selected".to_string()), Some(selection)),
                Err(error) => (false, Some(error.to_string()), None),
            };
            tracing::info!(
                session_id = %session_id.0,
                source_device_id = %source_device_id,
                accepted,
                "handled LAN capture source select"
            );
            let ack = LanDiscoveryPacket::CaptureSourceSelectAck {
                magic: DISCOVERY_MAGIC.to_string(),
                app_id: DISCOVERY_APP_ID.to_string(),
                instance_id: app_state.lan_discovery.instance_id.clone(),
                session_id: session_id.0,
                accepted,
                message,
                selection,
                timestamp_ms: now_ms(),
            };
            send_packet(socket, &ack, addr).await?;
        }
        LanDiscoveryPacket::CaptureSourceSelectAck { .. } => {}
    }

    Ok(())
}

async fn accept_lan_remote_session(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    source_device_id: DeviceId,
    transport_kind: String,
    requested_profile: Option<MediaProfile>,
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
    let negotiation = match negotiate_media_profile(requested_profile) {
        Ok(value) => value,
        Err(error) => return LanRemoteAcceptResult::rejected(error.to_string()),
    };

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
    app_state
        .media_profiles
        .lock()
        .await
        .set(session_id.clone(), negotiation.clone());
    spawn_quic_media_sender(app_state.clone(), session_id.clone(), listener);

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
        media_profile: Some(negotiation),
    }
}

async fn accept_lan_media_profile_update(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    requested_profile: MediaProfile,
) -> Result<MediaProfileNegotiation> {
    validate_media_profile(&requested_profile)?;
    {
        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions
            .get(session_id)
            .with_context(|| format!("session not found: {}", session_id.0))?;
        if normalize_transport_kind(&snapshot.transport) != "quic" {
            anyhow::bail!(
                "media profile update is only supported for LAN QUIC sessions, got {}",
                snapshot.transport
            );
        }
        if snapshot.lifecycle_state == "closed" || snapshot.lifecycle_state == "failed" {
            anyhow::bail!(
                "media profile update rejected for {} session",
                snapshot.lifecycle_state
            );
        }
    }

    let negotiation = negotiate_media_profile(Some(requested_profile))?;
    app_state
        .media_profiles
        .lock()
        .await
        .set(session_id.clone(), negotiation.clone());
    Ok(negotiation)
}

async fn accept_lan_capture_sources_request(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    include_previews: bool,
    limit: Option<u32>,
) -> Result<Vec<CaptureSource>> {
    ensure_active_sender_session(app_state, session_id, "capture source listing").await?;
    crate::capture_source::list_capture_sources(include_previews, limit)
}

async fn accept_lan_capture_source_select(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: &str,
) -> Result<CaptureSourceSelection> {
    let source = crate::capture_source::find_capture_source(source_id)?;
    accept_lan_capture_source_select_from_sources(app_state, session_id, source_id, vec![source])
        .await
}

async fn accept_lan_capture_source_select_from_sources(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_id: &str,
    sources: Vec<CaptureSource>,
) -> Result<CaptureSourceSelection> {
    ensure_active_sender_session(app_state, session_id, "capture source selection").await?;
    let source = sources
        .into_iter()
        .find(|source| source.id.eq_ignore_ascii_case(source_id))
        .with_context(|| format!("capture source not found: {source_id}"))?;
    let selection = CaptureSourceSelection {
        session_id: session_id.clone(),
        source,
        status: "selected".to_string(),
        reason: None,
    };
    app_state
        .capture_sources
        .lock()
        .await
        .set(session_id.clone(), selection.clone());
    Ok(selection)
}

async fn ensure_active_sender_session(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    operation: &str,
) -> Result<()> {
    let sessions = app_state.sessions.lock().await;
    let snapshot = sessions
        .get(session_id)
        .with_context(|| format!("session not found: {}", session_id.0))?;
    if normalize_transport_kind(&snapshot.transport) != "quic" {
        anyhow::bail!("{operation} is only supported for LAN QUIC sessions");
    }
    if !snapshot.sender_active {
        anyhow::bail!("{operation} requires an active target sender session");
    }
    if snapshot.lifecycle_state == "closed" || snapshot.lifecycle_state == "failed" {
        anyhow::bail!(
            "{operation} rejected for {} session",
            snapshot.lifecycle_state
        );
    }
    Ok(())
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
        transports: vec![
            "quic".to_string(),
            LAN_QUIC_MEDIA_TRANSPORT.to_string(),
            LAN_QUIC_MEDIA_PROFILE_TRANSPORT.to_string(),
            LAN_MEDIA_PROFILE_CONTROL_TRANSPORT.to_string(),
            LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT.to_string(),
        ],
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
    if transport == "quic" {
        let required = [
            LAN_QUIC_MEDIA_TRANSPORT,
            LAN_QUIC_MEDIA_PROFILE_TRANSPORT,
            LAN_MEDIA_PROFILE_CONTROL_TRANSPORT,
        ];
        let missing = required
            .iter()
            .filter(|required_transport| {
                !peer_transports
                    .iter()
                    .any(|peer_transport| peer_transport.eq_ignore_ascii_case(required_transport))
            })
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            anyhow::bail!(
                "LAN peer does not advertise required media controls [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
                missing.join(", "),
                target_device_id.0,
                format_peer_transports(peer_transports)
            );
        }
    }
    Ok(())
}

async fn peer_control_addr_with_capture_source_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
) -> Result<SocketAddr> {
    let target = app_state
        .lan_discovery
        .peer_control_addr(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    if !peer_transports
        .iter()
        .any(|transport| transport.eq_ignore_ascii_case(LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT))
    {
        anyhow::bail!(
            "LAN peer does not advertise required capture source control [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
            LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT,
            peer_device_id.0,
            format_peer_transports(&peer_transports)
        );
    }
    Ok(target)
}

async fn session_remote_peer(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Result<DeviceId> {
    let sessions = app_state.sessions.lock().await;
    let snapshot = sessions
        .get(session_id)
        .with_context(|| format!("session not found: {}", session_id.0))?;
    snapshot
        .target_device_id
        .clone()
        .or_else(|| snapshot.source_device_id.clone())
        .with_context(|| format!("session has no remote peer: {}", session_id.0))
}

async fn local_device_id(app_state: &Arc<AppState>) -> Result<String> {
    let devices = app_state.devices.lock().await;
    devices
        .get_local_device()
        .map(|(id, _)| id.0.clone())
        .context("local device is not registered")
}

fn format_peer_transports(peer_transports: &[String]) -> String {
    if peer_transports.is_empty() {
        "none".to_string()
    } else {
        peer_transports.join(", ")
    }
}

fn fit_capture_sources_ack_packet(
    instance_id: String,
    session_id: String,
    accepted: bool,
    message: Option<String>,
    sources: Vec<CaptureSource>,
) -> LanDiscoveryPacket {
    let mut packet = LanDiscoveryPacket::CaptureSourcesAck {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id,
        session_id,
        accepted,
        message,
        sources,
        timestamp_ms: now_ms(),
    };

    while serialized_packet_len(&packet) > DISCOVERY_SAFE_UDP_PAYLOAD_BYTES {
        let LanDiscoveryPacket::CaptureSourcesAck { sources, .. } = &mut packet else {
            break;
        };

        if let Some(index) = largest_preview_source_index(sources) {
            sources[index].preview_data_url = None;
            sources[index].preview_width = None;
            sources[index].preview_height = None;
            continue;
        }

        if sources.len() > 1 {
            sources.pop();
            continue;
        }

        break;
    }

    packet
}

fn serialized_packet_len(packet: &LanDiscoveryPacket) -> usize {
    serde_json::to_vec(packet)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn largest_preview_source_index(sources: &[CaptureSource]) -> Option<usize> {
    sources
        .iter()
        .enumerate()
        .filter_map(|(index, source)| {
            source
                .preview_data_url
                .as_ref()
                .map(|preview| (index, preview.len()))
        })
        .max_by_key(|(_, preview_len)| *preview_len)
        .map(|(index, _)| index)
}

fn spawn_quic_media_sender(
    app_state: Arc<AppState>,
    session_id: SessionId,
    listener: QuinnServerListener,
) {
    tokio::spawn(async move {
        let local_addr = listener.local_addr();
        let result = async move {
            let endpoint = listener
                .accept()
                .await
                .context("LAN QUIC media listener failed to accept receiver")?;
            send_quic_media_loop(app_state, endpoint, session_id).await
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(%error, %local_addr, "LAN QUIC media sender stopped");
        }
    });
}

async fn send_quic_media_loop(
    app_state: Arc<AppState>,
    endpoint: QuinnDatagramEndpoint,
    session_id: SessionId,
) -> Result<()> {
    let max_datagram_size = endpoint
        .max_datagram_size()
        .unwrap_or(LAN_QUIC_FALLBACK_DATAGRAM_BYTES)
        .min(LAN_QUIC_FALLBACK_DATAGRAM_BYTES)
        .max(QUIC_AU_FRAGMENT_HEADER_LEN + 1);

    let mut frame_id = 1_u64;
    loop {
        let profile = selected_media_profile(&app_state, &session_id).await;
        tokio::time::sleep(media_frame_interval(&profile)).await;
        let payload = build_media_probe_frame(frame_id, now_ms().saturating_mul(1_000), &profile);
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
        frame_id = frame_id.wrapping_add(1).max(1);
    }
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
            match decode_media_probe_frame(&frame.payload) {
                Ok(stats) => {
                    app_state.probes.lock().await.record_media_probe_frame(
                        &session_id,
                        stats,
                        now_ms(),
                    );
                }
                Err(error) => {
                    app_state.probes.lock().await.record_probe_drop(
                        &session_id,
                        frame.payload.len() as u64,
                        now_ms(),
                        error.to_string(),
                    );
                }
            }
        }
    }
}

fn build_media_probe_frame(sequence: u64, timestamp_us: u64, profile: &MediaProfile) -> Vec<u8> {
    let media_payload = build_probe_compressed_pattern(sequence, profile);
    let payload_hash = fnv1a64(&media_payload);
    let mut frame = Vec::with_capacity(LAN_MEDIA_PROBE_HEADER_BYTES + media_payload.len());
    frame.extend_from_slice(LAN_MEDIA_PROBE_MAGIC);
    frame.extend_from_slice(&sequence.to_le_bytes());
    frame.extend_from_slice(&timestamp_us.to_le_bytes());
    frame.extend_from_slice(&profile.width.to_le_bytes());
    frame.extend_from_slice(&profile.height.to_le_bytes());
    frame.extend_from_slice(&LAN_MEDIA_PROBE_FORMAT_CODE.to_le_bytes());
    frame.extend_from_slice(&(media_payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload_hash.to_le_bytes());
    frame.extend_from_slice(&profile.fps.to_le_bytes());
    frame.extend_from_slice(&profile.bitrate_mbps.to_le_bytes());
    frame.extend_from_slice(&media_payload);
    frame
}

fn decode_media_probe_frame(frame: &[u8]) -> Result<MediaProbeFrameStats> {
    if frame.len() < LAN_MEDIA_PROBE_HEADER_BYTES {
        anyhow::bail!("media probe frame is too small");
    }
    if &frame[..LAN_MEDIA_PROBE_MAGIC.len()] != LAN_MEDIA_PROBE_MAGIC {
        anyhow::bail!("media probe frame has invalid magic");
    }

    let sequence = u64::from_le_bytes(frame[8..16].try_into().unwrap());
    let timestamp_us = u64::from_le_bytes(frame[16..24].try_into().unwrap());
    let width = u32::from_le_bytes(frame[24..28].try_into().unwrap());
    let height = u32::from_le_bytes(frame[28..32].try_into().unwrap());
    let format_code = u32::from_le_bytes(frame[32..36].try_into().unwrap());
    let payload_len = u32::from_le_bytes(frame[36..40].try_into().unwrap()) as usize;
    let expected_hash = u64::from_le_bytes(frame[40..48].try_into().unwrap());
    let target_fps = u32::from_le_bytes(frame[48..52].try_into().unwrap());
    let target_bitrate_mbps = u32::from_le_bytes(frame[52..56].try_into().unwrap());

    let Some(expected_len) = LAN_MEDIA_PROBE_HEADER_BYTES.checked_add(payload_len) else {
        anyhow::bail!("media probe frame payload length overflow");
    };
    if frame.len() != expected_len {
        anyhow::bail!(
            "media probe frame payload length mismatch: expected {}, got {}",
            expected_len,
            frame.len()
        );
    }
    if format_code != LAN_MEDIA_PROBE_FORMAT_CODE {
        anyhow::bail!("unsupported media probe format code: {format_code}");
    }

    let media_payload = &frame[LAN_MEDIA_PROBE_HEADER_BYTES..];
    let actual_hash = fnv1a64(media_payload);
    if actual_hash != expected_hash {
        anyhow::bail!("media probe payload hash mismatch");
    }

    Ok(MediaProbeFrameStats {
        bytes_received: frame.len() as u64,
        sequence,
        timestamp_us,
        width,
        height,
        target_fps,
        target_bitrate_mbps,
        payload_bytes: payload_len as u32,
        format: media_probe_format(width, height, target_fps, target_bitrate_mbps).to_string(),
        payload_hash: format!("fnv1a64:{actual_hash:016x}"),
    })
}

fn build_probe_compressed_pattern(sequence: u64, profile: &MediaProfile) -> Vec<u8> {
    let mut payload = vec![0_u8; media_payload_bytes(profile)];
    for (offset, byte) in payload.iter_mut().enumerate() {
        let lane = (offset as u64).wrapping_mul(31);
        *byte = lane
            .wrapping_add(sequence.wrapping_mul(17))
            .wrapping_add((offset as u64 >> 8) * 13) as u8;
    }
    payload
}

async fn selected_media_profile(app_state: &Arc<AppState>, session_id: &SessionId) -> MediaProfile {
    app_state
        .media_profiles
        .lock()
        .await
        .get(session_id)
        .map(|negotiation| negotiation.selected)
        .unwrap_or_else(default_media_profile)
}

fn default_media_profile() -> MediaProfile {
    MediaProfile {
        width: LAN_MEDIA_TARGET_WIDTH,
        height: LAN_MEDIA_TARGET_HEIGHT,
        fps: LAN_MEDIA_TARGET_FPS,
        bitrate_mbps: LAN_MEDIA_TARGET_BITRATE_MBPS,
        codec: "h264".to_string(),
    }
}

fn default_media_profile_negotiation() -> MediaProfileNegotiation {
    let profile = default_media_profile();
    MediaProfileNegotiation {
        requested: profile.clone(),
        selected: profile,
        status: "accepted".to_string(),
        reason: None,
    }
}

fn negotiate_media_profile(
    requested_profile: Option<MediaProfile>,
) -> Result<MediaProfileNegotiation> {
    let requested = requested_profile.unwrap_or_else(default_media_profile);
    validate_media_profile(&requested)?;

    let mut selected = requested.clone();
    selected.width = selected.width.min(LAN_MEDIA_TARGET_WIDTH);
    selected.height = selected.height.min(LAN_MEDIA_TARGET_HEIGHT);
    selected.fps = selected.fps.min(LAN_MEDIA_TARGET_FPS);
    selected.bitrate_mbps = selected.bitrate_mbps.min(LAN_MEDIA_TARGET_BITRATE_MBPS);
    selected.codec = "h264".to_string();

    let changed = selected != requested;
    Ok(MediaProfileNegotiation {
        requested,
        selected,
        status: if changed { "downgraded" } else { "accepted" }.to_string(),
        reason: if changed {
            Some("clamped to LAN QUIC media capability".to_string())
        } else {
            None
        },
    })
}

fn validate_media_profile(profile: &MediaProfile) -> Result<()> {
    if profile.width == 0 || profile.height == 0 || profile.fps == 0 || profile.bitrate_mbps == 0 {
        anyhow::bail!("media profile width, height, fps and bitrate must be greater than zero");
    }
    Ok(())
}

fn media_frame_interval(profile: &MediaProfile) -> Duration {
    Duration::from_micros((1_000_000 / u64::from(profile.fps.max(1))).max(1))
}

fn media_payload_bytes(profile: &MediaProfile) -> usize {
    ((profile.bitrate_mbps as usize * 1_000_000 / 8) / profile.fps.max(1) as usize).max(1)
}

fn media_probe_format(
    width: u32,
    height: u32,
    target_fps: u32,
    target_bitrate_mbps: u32,
) -> &'static str {
    if width == LAN_MEDIA_TARGET_WIDTH
        && height == LAN_MEDIA_TARGET_HEIGHT
        && target_fps == LAN_MEDIA_TARGET_FPS
        && target_bitrate_mbps == LAN_MEDIA_TARGET_BITRATE_MBPS
    {
        LAN_MEDIA_PROBE_2K144_FORMAT
    } else {
        LAN_MEDIA_PROBE_DYNAMIC_FORMAT
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
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
            requested_media_profile: Some(MediaProfile {
                width: 3840,
                height: 2160,
                fps: 240,
                bitrate_mbps: 120,
                codec: "hevc".to_string(),
            }),
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
                media_profile,
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
                let negotiation = media_profile.expect("media profile negotiation");
                assert_eq!(negotiation.status, "downgraded");
                assert_eq!(negotiation.selected.width, LAN_MEDIA_TARGET_WIDTH);
                assert_eq!(negotiation.selected.height, LAN_MEDIA_TARGET_HEIGHT);
                assert_eq!(negotiation.selected.fps, LAN_MEDIA_TARGET_FPS);
                assert_eq!(
                    negotiation.selected.bitrate_mbps,
                    LAN_MEDIA_TARGET_BITRATE_MBPS
                );
                assert_eq!(negotiation.selected.codec, "h264");
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
            requested_media_profile: None,
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
                    transports: vec![
                        "quic".to_string(),
                        LAN_QUIC_MEDIA_TRANSPORT.to_string(),
                        LAN_QUIC_MEDIA_PROFILE_TRANSPORT.to_string(),
                        LAN_MEDIA_PROFILE_CONTROL_TRANSPORT.to_string(),
                    ],
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
            None,
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
        assert!(snapshot.media_probe_valid);
        assert_eq!(
            snapshot.media_probe_format.as_deref(),
            Some(LAN_MEDIA_PROBE_2K144_FORMAT)
        );
        assert_eq!(snapshot.media_probe_width, Some(LAN_MEDIA_TARGET_WIDTH));
        assert_eq!(snapshot.media_probe_height, Some(LAN_MEDIA_TARGET_HEIGHT));
        assert!(snapshot.last_media_sequence.unwrap_or_default() > 0);
        assert!(snapshot
            .last_media_payload_hash
            .as_deref()
            .unwrap_or_default()
            .starts_with("fnv1a64:"));
        assert_eq!(snapshot.media_probe_target_fps, Some(144));
        assert_eq!(snapshot.media_probe_target_bitrate_mbps, Some(64));
        assert!(snapshot.media_probe_payload_bytes.unwrap_or_default() > 0);
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
            None,
        )
        .await
        .expect_err("legacy QUIC peer should fail before session request");

        assert!(error.to_string().contains("quic_datagram"));
        assert!(error.to_string().contains("Rebuild and restart"));
    }

    #[tokio::test]
    async fn request_lan_remote_session_rejects_peer_without_2k144_media_profile() {
        let controller_state = Arc::new(AppState::new());
        controller_state.devices.lock().await.register(
            DeviceId("controller-device".to_string()),
            "Controller Device".to_string(),
        );

        let peer_addr: SocketAddr = "127.0.0.1:32217".parse().unwrap();
        controller_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "stale-target-instance".to_string(),
                    device_id: "stale-target-device".to_string(),
                    device_name: "Stale Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: 1,
                    discovery_port: peer_addr.port(),
                    transports: vec!["quic".to_string(), LAN_QUIC_MEDIA_TRANSPORT.to_string()],
                    timestamp_ms: now_ms(),
                },
                peer_addr,
            )
            .await;

        let error = request_lan_remote_session(
            &controller_state,
            &DeviceId("stale-target-device".to_string()),
            &SessionId("session-stale-peer".to_string()),
            "quic",
            None,
        )
        .await
        .expect_err("stale QUIC datagram peer should fail before session request");

        assert!(error.to_string().contains("quic_datagram_2k144"));
        assert!(error.to_string().contains("Rebuild and restart"));
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

    #[test]
    fn media_probe_frame_uses_2k144_compressed_profile() {
        let profile = default_media_profile();
        let frame = build_media_probe_frame(42, 123_456, &profile);
        let stats = decode_media_probe_frame(&frame).unwrap();

        assert_eq!(stats.sequence, 42);
        assert_eq!(stats.width, 2560);
        assert_eq!(stats.height, 1440);
        assert_eq!(stats.target_fps, 144);
        assert_eq!(stats.target_bitrate_mbps, 64);
        assert_eq!(stats.format, "compressed_2k144_test_pattern");
        assert!(stats.bytes_received < (2560_u64 * 1440 * 4));
        assert!(stats.payload_hash.starts_with("fnv1a64:"));
    }

    #[test]
    fn media_profile_negotiation_clamps_to_lan_capability() {
        let negotiation = negotiate_media_profile(Some(MediaProfile {
            width: 3840,
            height: 2160,
            fps: 240,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
        }))
        .unwrap();

        assert_eq!(negotiation.status, "downgraded");
        assert_eq!(negotiation.selected.width, 2560);
        assert_eq!(negotiation.selected.height, 1440);
        assert_eq!(negotiation.selected.fps, 144);
        assert_eq!(negotiation.selected.bitrate_mbps, 64);
        assert_eq!(negotiation.selected.codec, "h264");
    }

    #[tokio::test]
    async fn capture_source_selection_changes_active_sender_session() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("capture-source-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let source = mrd_ipc::CaptureSource {
            id: "windows:window:0x1234".to_string(),
            platform: "windows".to_string(),
            source_kind: "window".to_string(),
            title: "Target App".to_string(),
            class_name: "ApplicationFrameWindow".to_string(),
            width: 1280,
            height: 720,
            process_id: 4242,
            app_name: Some("Target App".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        };
        let selection = accept_lan_capture_source_select_from_sources(
            &app_state,
            &session_id,
            "windows:window:0x1234",
            vec![source],
        )
        .await
        .unwrap();

        assert_eq!(selection.status, "selected");
        assert_eq!(selection.source.id, "windows:window:0x1234");
        assert_eq!(
            app_state
                .capture_sources
                .lock()
                .await
                .get(&session_id)
                .expect("selected capture source")
                .source
                .source_kind,
            "window"
        );
    }

    #[test]
    fn capture_sources_ack_trims_preview_payload_to_udp_budget() {
        let sources = (0..24)
            .map(|index| mrd_ipc::CaptureSource {
                id: format!("windows:window:0x{:X}", index + 0x1000),
                platform: "windows".to_string(),
                source_kind: "window".to_string(),
                title: format!("Target App {index}"),
                class_name: "ApplicationFrameWindow".to_string(),
                width: 1280,
                height: 720,
                process_id: 4242 + index,
                app_name: Some(format!("Target App {index}")),
                bundle_identifier: None,
                preview_data_url: Some(format!("data:image/png;base64,{}", "A".repeat(8_000))),
                preview_width: Some(240),
                preview_height: Some(135),
            })
            .collect();

        let packet = fit_capture_sources_ack_packet(
            "target-instance".to_string(),
            "capture-source-session".to_string(),
            true,
            Some("listed".to_string()),
            sources,
        );

        assert!(serialized_packet_len(&packet) <= DISCOVERY_SAFE_UDP_PAYLOAD_BYTES);
        let LanDiscoveryPacket::CaptureSourcesAck { sources, .. } = packet else {
            panic!("expected capture sources ack");
        };
        assert!(sources
            .iter()
            .any(|source| source.preview_data_url.is_some()));
        assert!(sources
            .iter()
            .any(|source| source.preview_data_url.is_none()));
    }

    #[test]
    fn dynamic_media_probe_frame_preserves_selected_profile() {
        let profile = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "h264".to_string(),
        };
        let frame = build_media_probe_frame(7, 99_000, &profile);
        let stats = decode_media_probe_frame(&frame).unwrap();

        assert_eq!(stats.width, 1920);
        assert_eq!(stats.height, 1080);
        assert_eq!(stats.target_fps, 60);
        assert_eq!(stats.target_bitrate_mbps, 20);
        assert_eq!(stats.payload_bytes, media_payload_bytes(&profile) as u32);
        assert_eq!(stats.format, "compressed_h264_test_pattern");
    }

    #[tokio::test]
    async fn media_profile_update_changes_active_quic_session_profile() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("profile-update-session".to_string());
        app_state.sessions.lock().await.insert(
            session_id.clone(),
            SessionSnapshot {
                session_id: session_id.clone(),
                transport: "quic".to_string(),
                source_device_id: Some(DeviceId("controller-device".to_string())),
                target_device_id: None,
                local_listen_addr: None,
                local_server_name: None,
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

        let negotiation = accept_lan_media_profile_update(
            &app_state,
            &session_id,
            MediaProfile {
                width: 1280,
                height: 720,
                fps: 60,
                bitrate_mbps: 8,
                codec: "h264".to_string(),
            },
        )
        .await
        .unwrap();

        assert_eq!(negotiation.status, "accepted");
        assert_eq!(negotiation.selected.width, 1280);
        assert_eq!(
            app_state
                .media_profiles
                .lock()
                .await
                .get(&session_id)
                .expect("profile update result")
                .selected
                .height,
            720
        );
    }
}
