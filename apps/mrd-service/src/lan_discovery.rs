use crate::app_state::AppState;
use anyhow::{Context, Result};
use mrd_application::ports::SessionSnapshot;
use mrd_ipc::{LanDiscoverySnapshot, LanPeerInfo};
use mrd_proto::{DeviceId, SessionId};
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
        timestamp_ms: now_ms(),
    };
    send_packet(&socket, &packet, target).await?;

    let mut buffer = vec![0_u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut buffer))
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
            ..
        } if is_valid_discovery_packet(&magic, &app_id) && ack_session_id == session_id.0 => {
            if accepted {
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
    let mut buffer = vec![0_u8; 2048];
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

            let (accepted, message) = accept_lan_remote_session(
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
                session_id,
                accepted,
                message,
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
) -> (bool, Option<String>) {
    let is_registered = {
        let devices = app_state.devices.lock().await;
        devices.is_registered()
    };
    if !is_registered {
        return (false, Some("local device is not registered".to_string()));
    }

    let mut sessions = app_state.sessions.lock().await;
    sessions.insert(
        session_id.clone(),
        SessionSnapshot {
            session_id,
            transport: if transport_kind.trim().is_empty() {
                "webrtc".to_string()
            } else {
                transport_kind
            },
            source_device_id: Some(source_device_id),
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

    (true, Some("accepted".to_string()))
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
        transports: vec!["webrtc".to_string(), "quic".to_string()],
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

        let mut buffer = vec![0_u8; 2048];
        let (len, _) = timeout(Duration::from_secs(1), ack_socket.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len]).unwrap();
        match ack {
            LanDiscoveryPacket::RemoteSessionAck {
                session_id,
                accepted,
                ..
            } => {
                assert_eq!(session_id, "session-1");
                assert!(accepted);
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

    #[test]
    fn discovery_packet_requires_rdesk_namespace() {
        assert!(is_valid_discovery_packet(DISCOVERY_MAGIC, DISCOVERY_APP_ID));
        assert!(!is_valid_discovery_packet(DISCOVERY_MAGIC, "rsharemouse"));
    }
}
