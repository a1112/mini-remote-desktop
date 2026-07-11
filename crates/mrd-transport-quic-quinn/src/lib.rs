pub mod low_latency;
pub mod recovery;

use std::{
    collections::{BTreeMap, HashMap},
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig};
use ring::digest;
use rustls::RootCertStore;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

const QUIC_SERVER_HANDSHAKE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicTransportMetadata {
    pub transport: &'static str,
    pub local_addr: SocketAddr,
    pub peer_addr: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuinnServerBootstrap {
    pub transport: &'static str,
    pub listen_addr: SocketAddr,
    pub server_name: String,
    pub cert_der: Vec<u8>,
}

impl QuinnServerBootstrap {
    /// SHA-256 commitment to the exact ephemeral leaf certificate used by this listener.
    pub fn certificate_fingerprint_sha256(&self) -> [u8; 32] {
        certificate_fingerprint_sha256(&self.cert_der)
    }
}

/// Computes a stable SHA-256 commitment for an encoded X.509 certificate.
pub fn certificate_fingerprint_sha256(cert_der: &[u8]) -> [u8; 32] {
    let mut fingerprint = [0_u8; 32];
    fingerprint.copy_from_slice(digest::digest(&digest::SHA256, cert_der).as_ref());
    fingerprint
}

pub struct QuinnServerListener {
    endpoint: Endpoint,
    local_addr: SocketAddr,
}

impl QuinnServerListener {
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn bind(
        bind_addr: &str,
    ) -> Result<(Self, QuinnServerBootstrap), QuinnTransportError> {
        let server_crypto =
            rcgen::generate_simple_self_signed(vec!["localhost".into()]).map_err(|error| {
                QuinnTransportError::Message(format!("generate cert failed: {error}"))
            })?;
        let server_cert = rustls::pki_types::CertificateDer::from(server_crypto.cert);
        let cert_der = server_cert.as_ref().to_vec();
        let server_key =
            rustls::pki_types::PrivatePkcs8KeyDer::from(server_crypto.signing_key.serialize_der());
        let server_config = ServerConfig::with_single_cert(
            vec![server_cert],
            rustls::pki_types::PrivateKeyDer::Pkcs8(server_key),
        )
        .map_err(|error| QuinnTransportError::Message(format!("server config failed: {error}")))?;

        let bind_addr = bind_addr.parse::<SocketAddr>().map_err(|error| {
            QuinnTransportError::Message(format!("parse bind_addr failed: {error}"))
        })?;
        let endpoint = Endpoint::server(server_config, bind_addr).map_err(|error| {
            QuinnTransportError::Message(format!("server endpoint failed: {error}"))
        })?;
        let local_addr = endpoint.local_addr().map_err(|error| {
            QuinnTransportError::Message(format!("server local_addr failed: {error}"))
        })?;

        Ok((
            Self {
                endpoint,
                local_addr,
            },
            QuinnServerBootstrap {
                transport: "quic_quinn",
                listen_addr: local_addr,
                server_name: "localhost".into(),
                cert_der,
            },
        ))
    }

    pub async fn accept(self) -> Result<QuinnDatagramEndpoint, QuinnTransportError> {
        self.accept_matching_ip(None).await
    }

    /// Accepts only a QUIC client from the IP that sent the authenticated session request.
    pub async fn accept_from(
        self,
        expected_peer_ip: IpAddr,
    ) -> Result<QuinnDatagramEndpoint, QuinnTransportError> {
        self.accept_matching_ip(Some(expected_peer_ip)).await
    }

    async fn accept_matching_ip(
        self,
        expected_peer_ip: Option<IpAddr>,
    ) -> Result<QuinnDatagramEndpoint, QuinnTransportError> {
        loop {
            let connecting = self.endpoint.accept().await.ok_or_else(|| {
                QuinnTransportError::Message("server accept returned None".into())
            })?;
            let connection =
                match tokio::time::timeout(QUIC_SERVER_HANDSHAKE_ATTEMPT_TIMEOUT, connecting).await
                {
                    Ok(Ok(connection)) => connection,
                    Ok(Err(_)) | Err(_) => continue,
                };
            let peer_addr = connection.remote_address();
            if expected_peer_ip.is_some_and(|expected| peer_addr.ip() != expected) {
                connection.close(0_u32.into(), b"unexpected authenticated peer address");
                continue;
            }
            let metadata = QuicTransportMetadata {
                transport: "quic_quinn",
                local_addr: self.local_addr,
                peer_addr,
            };

            return Ok(QuinnDatagramEndpoint {
                inner: Arc::new(QuinnDatagramEndpointInner {
                    endpoint: self.endpoint,
                    connection,
                    metadata,
                    reliable_send_stream: Mutex::new(None),
                    reliable_recv_stream: Mutex::new(None),
                }),
            });
        }
    }
}

#[derive(Debug)]
struct QuinnDatagramEndpointInner {
    endpoint: Endpoint,
    connection: Connection,
    metadata: QuicTransportMetadata,
    reliable_send_stream: Mutex<Option<quinn::SendStream>>,
    reliable_recv_stream: Mutex<Option<quinn::RecvStream>>,
}

impl Drop for QuinnDatagramEndpointInner {
    fn drop(&mut self) {
        self.connection.close(0_u32.into(), b"shutdown");
        self.endpoint.close(0_u32.into(), b"shutdown");
    }
}

#[derive(Debug, Clone)]
pub struct QuinnDatagramEndpoint {
    inner: Arc<QuinnDatagramEndpointInner>,
}

impl QuinnDatagramEndpoint {
    pub fn metadata(&self) -> &QuicTransportMetadata {
        &self.inner.metadata
    }

    pub fn max_datagram_size(&self) -> Option<usize> {
        self.inner.connection.max_datagram_size()
    }

    pub fn send_datagram(&self, payload: Bytes) -> Result<(), QuinnTransportError> {
        self.inner
            .connection
            .send_datagram(payload)
            .map_err(|error| QuinnTransportError::Message(format!("send_datagram failed: {error}")))
    }

    pub async fn send_datagram_wait(&self, payload: Bytes) -> Result<(), QuinnTransportError> {
        self.inner
            .connection
            .send_datagram_wait(payload)
            .await
            .map_err(|error| {
                QuinnTransportError::Message(format!("send_datagram_wait failed: {error}"))
            })
    }

    pub async fn read_datagram(&self) -> Result<Bytes, QuinnTransportError> {
        self.inner
            .connection
            .read_datagram()
            .await
            .map_err(|error| QuinnTransportError::Message(format!("read_datagram failed: {error}")))
    }

    pub async fn send_reliable_message(&self, payload: Bytes) -> Result<(), QuinnTransportError> {
        let mut stream =
            self.inner.connection.open_uni().await.map_err(|error| {
                QuinnTransportError::Message(format!("open_uni failed: {error}"))
            })?;
        stream.write_all(payload.as_ref()).await.map_err(|error| {
            QuinnTransportError::Message(format!("reliable stream write failed: {error}"))
        })?;
        stream.finish().map_err(|error| {
            QuinnTransportError::Message(format!("reliable stream finish failed: {error}"))
        })
    }

    pub async fn read_reliable_message(
        &self,
        max_len: usize,
    ) -> Result<Bytes, QuinnTransportError> {
        let mut stream =
            self.inner.connection.accept_uni().await.map_err(|error| {
                QuinnTransportError::Message(format!("accept_uni failed: {error}"))
            })?;
        let payload = stream.read_to_end(max_len).await.map_err(|error| {
            QuinnTransportError::Message(format!("reliable stream read failed: {error}"))
        })?;
        Ok(Bytes::from(payload))
    }

    pub async fn send_reliable_message_persistent(
        &self,
        payload: Bytes,
    ) -> Result<(), QuinnTransportError> {
        let payload_len = u32::try_from(payload.len()).map_err(|_| {
            QuinnTransportError::Message(format!(
                "persistent reliable payload too large: {} bytes",
                payload.len()
            ))
        })?;
        let mut stream_guard = self.inner.reliable_send_stream.lock().await;
        if stream_guard.is_none() {
            let stream = self.inner.connection.open_uni().await.map_err(|error| {
                QuinnTransportError::Message(format!("open persistent uni failed: {error}"))
            })?;
            *stream_guard = Some(stream);
        }

        let stream = stream_guard
            .as_mut()
            .ok_or_else(|| QuinnTransportError::Message("persistent uni stream missing".into()))?;
        if let Err(error) = stream.write_all(&payload_len.to_le_bytes()).await {
            *stream_guard = None;
            return Err(QuinnTransportError::Message(format!(
                "persistent reliable length write failed: {error}"
            )));
        }
        if let Err(error) = stream.write_all(payload.as_ref()).await {
            *stream_guard = None;
            return Err(QuinnTransportError::Message(format!(
                "persistent reliable payload write failed: {error}"
            )));
        }
        if let Err(error) = stream.flush().await {
            *stream_guard = None;
            return Err(QuinnTransportError::Message(format!(
                "persistent reliable flush failed: {error}"
            )));
        }
        Ok(())
    }

    pub async fn read_reliable_message_persistent(
        &self,
        max_len: usize,
    ) -> Result<Bytes, QuinnTransportError> {
        let mut stream_guard = self.inner.reliable_recv_stream.lock().await;
        if stream_guard.is_none() {
            let stream = self.inner.connection.accept_uni().await.map_err(|error| {
                QuinnTransportError::Message(format!("accept persistent uni failed: {error}"))
            })?;
            *stream_guard = Some(stream);
        }

        let stream = stream_guard.as_mut().ok_or_else(|| {
            QuinnTransportError::Message("persistent receive stream missing".into())
        })?;
        let mut header = [0_u8; 4];
        if let Err(error) = stream.read_exact(&mut header).await {
            *stream_guard = None;
            return Err(QuinnTransportError::Message(format!(
                "persistent reliable length read failed: {error}"
            )));
        }
        let payload_len = u32::from_le_bytes(header) as usize;
        if payload_len > max_len {
            *stream_guard = None;
            return Err(QuinnTransportError::Message(format!(
                "persistent reliable payload too large: {payload_len} > {max_len}"
            )));
        }
        let mut payload = vec![0_u8; payload_len];
        if let Err(error) = stream.read_exact(&mut payload).await {
            *stream_guard = None;
            return Err(QuinnTransportError::Message(format!(
                "persistent reliable payload read failed: {error}"
            )));
        }
        Ok(Bytes::from(payload))
    }

    pub async fn connect_client(
        bind_addr: &str,
        bootstrap: &QuinnServerBootstrap,
    ) -> Result<Self, QuinnTransportError> {
        let mut roots = RootCertStore::empty();
        let server_cert = rustls::pki_types::CertificateDer::from(bootstrap.cert_der.clone());
        roots.add(server_cert).map_err(|error| {
            QuinnTransportError::Message(format!("add root cert failed: {error}"))
        })?;
        let client_config =
            ClientConfig::with_root_certificates(Arc::new(roots)).map_err(|error| {
                QuinnTransportError::Message(format!("client config failed: {error}"))
            })?;

        let bind_addr = bind_addr.parse::<SocketAddr>().map_err(|error| {
            QuinnTransportError::Message(format!("parse bind_addr failed: {error}"))
        })?;
        let mut client_endpoint = Endpoint::client(bind_addr).map_err(|error| {
            QuinnTransportError::Message(format!("client endpoint failed: {error}"))
        })?;
        client_endpoint.set_default_client_config(client_config);
        let client_connection = client_endpoint
            .connect(bootstrap.listen_addr, &bootstrap.server_name)
            .map_err(|error| QuinnTransportError::Message(format!("connect failed: {error}")))?
            .await
            .map_err(|error| {
                QuinnTransportError::Message(format!("client handshake failed: {error}"))
            })?;
        let metadata = QuicTransportMetadata {
            transport: bootstrap.transport,
            local_addr: client_endpoint.local_addr().map_err(|error| {
                QuinnTransportError::Message(format!("client local_addr failed: {error}"))
            })?,
            peer_addr: client_connection.remote_address(),
        };

        Ok(Self {
            inner: Arc::new(QuinnDatagramEndpointInner {
                endpoint: client_endpoint,
                connection: client_connection,
                metadata,
                reliable_send_stream: Mutex::new(None),
                reliable_recv_stream: Mutex::new(None),
            }),
        })
    }
}

pub struct QuinnDatagramPair {
    pub client: QuinnDatagramEndpoint,
    pub server: QuinnDatagramEndpoint,
}

impl QuinnDatagramPair {
    pub async fn loopback() -> Result<Self, QuinnTransportError> {
        let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0").await?;
        let server_task = tokio::spawn(async move { listener.accept().await });
        let client = QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &bootstrap).await?;
        let server = server_task.await.map_err(|error| {
            QuinnTransportError::Message(format!("server task join failed: {error}"))
        })??;

        Ok(Self { client, server })
    }
}

pub const QUIC_AU_FRAGMENT_HEADER_LEN: usize = 17;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicAuFrame {
    pub frame_id: u32,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
    pub payload: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicAuFragment {
    pub frame_id: u32,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
    pub fragment_index: u16,
    pub fragment_count: u16,
    pub payload: Bytes,
}

impl QuicAuFragment {
    pub fn encode(&self) -> Bytes {
        let mut buffer = BytesMut::with_capacity(QUIC_AU_FRAGMENT_HEADER_LEN + self.payload.len());
        buffer.put_u32_le(self.frame_id);
        buffer.put_u64_le(self.timestamp_us);
        buffer.put_u8(u8::from(self.is_keyframe));
        buffer.put_u16_le(self.fragment_index);
        buffer.put_u16_le(self.fragment_count);
        buffer.extend_from_slice(&self.payload);
        buffer.freeze()
    }

    pub fn decode(datagram: &[u8]) -> Result<Self, QuinnTransportError> {
        if datagram.len() < QUIC_AU_FRAGMENT_HEADER_LEN {
            return Err(QuinnTransportError::Message("datagram too small".into()));
        }
        let mut bytes = datagram;
        let frame_id = bytes.get_u32_le();
        let timestamp_us = bytes.get_u64_le();
        let is_keyframe = bytes.get_u8() != 0;
        let fragment_index = bytes.get_u16_le();
        let fragment_count = bytes.get_u16_le();
        if fragment_count == 0 {
            return Err(QuinnTransportError::Message(
                "fragment_count must be non-zero".into(),
            ));
        }
        if fragment_index >= fragment_count {
            return Err(QuinnTransportError::Message(
                "fragment_index out of range".into(),
            ));
        }
        Ok(Self {
            frame_id,
            timestamp_us,
            is_keyframe,
            fragment_index,
            fragment_count,
            payload: Bytes::copy_from_slice(bytes),
        })
    }
}

pub fn fragment_access_unit(
    frame_id: u32,
    timestamp_us: u64,
    is_keyframe: bool,
    payload: &[u8],
    max_datagram_size: usize,
) -> Result<Vec<Bytes>, QuinnTransportError> {
    if max_datagram_size <= QUIC_AU_FRAGMENT_HEADER_LEN {
        return Err(QuinnTransportError::Message(
            "max_datagram_size too small for fragment header".into(),
        ));
    }
    let max_fragment_payload = max_datagram_size - QUIC_AU_FRAGMENT_HEADER_LEN;
    let fragment_count = payload.len().div_ceil(max_fragment_payload).max(1);
    if fragment_count > u16::MAX as usize {
        return Err(QuinnTransportError::Message(
            "fragment_count exceeds u16 range".into(),
        ));
    }

    let mut fragments = Vec::with_capacity(fragment_count);
    for (fragment_index, chunk) in payload.chunks(max_fragment_payload).enumerate() {
        fragments.push(
            QuicAuFragment {
                frame_id,
                timestamp_us,
                is_keyframe,
                fragment_index: fragment_index as u16,
                fragment_count: fragment_count as u16,
                payload: Bytes::copy_from_slice(chunk),
            }
            .encode(),
        );
    }
    if fragments.is_empty() {
        fragments.push(
            QuicAuFragment {
                frame_id,
                timestamp_us,
                is_keyframe,
                fragment_index: 0,
                fragment_count: 1,
                payload: Bytes::new(),
            }
            .encode(),
        );
    }
    Ok(fragments)
}

pub const QUIC_MEDIA_V3_MAGIC: u32 = 0x3344_524d;
pub const QUIC_MEDIA_V3_VERSION: u8 = 3;
pub const QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN: usize = 32;
pub const QUIC_MEDIA_V3_FLAG_KEYFRAME: u8 = 0x01;

pub fn is_quic_media_v3_datagram(datagram: &[u8]) -> bool {
    if datagram.len() < QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN {
        return false;
    }
    let magic = u32::from_le_bytes(datagram[0..4].try_into().unwrap());
    let version = datagram[4];
    magic == QUIC_MEDIA_V3_MAGIC && version == QUIC_MEDIA_V3_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum QuicMediaPayloadType {
    AccessUnit = 1,
    Probe = 2,
    Control = 3,
}

impl QuicMediaPayloadType {
    fn decode(value: u8) -> Result<Self, QuinnTransportError> {
        match value {
            1 => Ok(Self::AccessUnit),
            2 => Ok(Self::Probe),
            3 => Ok(Self::Control),
            _ => Err(QuinnTransportError::Message(format!(
                "unknown media v3 payload type {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum QuicMediaCodec {
    None = 0,
    H264 = 1,
    Hevc = 2,
    Av1 = 3,
}

impl QuicMediaCodec {
    fn decode(value: u8) -> Result<Self, QuinnTransportError> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::H264),
            2 => Ok(Self::Hevc),
            3 => Ok(Self::Av1),
            _ => Err(QuinnTransportError::Message(format!(
                "unknown media v3 codec {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicMediaFrame {
    pub payload_type: QuicMediaPayloadType,
    pub codec: QuicMediaCodec,
    pub profile_id: u32,
    pub frame_id: u32,
    pub timestamp_us: u64,
    pub flags: u8,
    pub payload: Bytes,
}

impl QuicMediaFrame {
    pub fn is_keyframe(&self) -> bool {
        self.flags & QUIC_MEDIA_V3_FLAG_KEYFRAME != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicMediaFragment {
    pub payload_type: QuicMediaPayloadType,
    pub codec: QuicMediaCodec,
    pub profile_id: u32,
    pub frame_id: u32,
    pub timestamp_us: u64,
    pub flags: u8,
    pub fragment_index: u16,
    pub fragment_count: u16,
    pub payload: Bytes,
}

impl QuicMediaFragment {
    pub fn encode(&self) -> Bytes {
        let mut buffer =
            BytesMut::with_capacity(QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN + self.payload.len());
        buffer.put_u32_le(QUIC_MEDIA_V3_MAGIC);
        buffer.put_u8(QUIC_MEDIA_V3_VERSION);
        buffer.put_u8(self.payload_type as u8);
        buffer.put_u8(self.codec as u8);
        buffer.put_u8(self.flags);
        buffer.put_u32_le(self.profile_id);
        buffer.put_u32_le(self.frame_id);
        buffer.put_u64_le(self.timestamp_us);
        buffer.put_u16_le(self.fragment_index);
        buffer.put_u16_le(self.fragment_count);
        buffer.put_u32_le(self.payload.len() as u32);
        buffer.extend_from_slice(&self.payload);
        buffer.freeze()
    }

    pub fn decode(datagram: &[u8]) -> Result<Self, QuinnTransportError> {
        if datagram.len() < QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN {
            return Err(QuinnTransportError::Message(
                "media v3 datagram too small".into(),
            ));
        }
        let mut bytes = datagram;
        let magic = bytes.get_u32_le();
        if magic != QUIC_MEDIA_V3_MAGIC {
            return Err(QuinnTransportError::Message(
                "invalid media v3 magic".into(),
            ));
        }
        let version = bytes.get_u8();
        if version != QUIC_MEDIA_V3_VERSION {
            return Err(QuinnTransportError::Message(format!(
                "unsupported media v3 version {version}"
            )));
        }
        let payload_type = QuicMediaPayloadType::decode(bytes.get_u8())?;
        let codec = QuicMediaCodec::decode(bytes.get_u8())?;
        let flags = bytes.get_u8();
        let profile_id = bytes.get_u32_le();
        let frame_id = bytes.get_u32_le();
        let timestamp_us = bytes.get_u64_le();
        let fragment_index = bytes.get_u16_le();
        let fragment_count = bytes.get_u16_le();
        let payload_len = bytes.get_u32_le() as usize;
        if fragment_count == 0 {
            return Err(QuinnTransportError::Message(
                "media v3 fragment_count must be non-zero".into(),
            ));
        }
        if fragment_index >= fragment_count {
            return Err(QuinnTransportError::Message(
                "media v3 fragment_index out of range".into(),
            ));
        }
        if payload_len != bytes.remaining() {
            return Err(QuinnTransportError::Message(
                "media v3 payload length mismatch".into(),
            ));
        }
        Ok(Self {
            payload_type,
            codec,
            profile_id,
            frame_id,
            timestamp_us,
            flags,
            fragment_index,
            fragment_count,
            payload: Bytes::copy_from_slice(bytes),
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub fn fragment_media_payload_v3(
    payload_type: QuicMediaPayloadType,
    codec: QuicMediaCodec,
    profile_id: u32,
    frame_id: u32,
    timestamp_us: u64,
    is_keyframe: bool,
    payload: &[u8],
    max_datagram_size: usize,
) -> Result<Vec<Bytes>, QuinnTransportError> {
    if max_datagram_size <= QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN {
        return Err(QuinnTransportError::Message(
            "max_datagram_size too small for media v3 fragment header".into(),
        ));
    }
    let max_fragment_payload = max_datagram_size - QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN;
    let fragment_count = payload.len().div_ceil(max_fragment_payload).max(1);
    if fragment_count > u16::MAX as usize {
        return Err(QuinnTransportError::Message(
            "media v3 fragment_count exceeds u16 range".into(),
        ));
    }

    let flags = if is_keyframe {
        QUIC_MEDIA_V3_FLAG_KEYFRAME
    } else {
        0
    };
    let mut fragments = Vec::with_capacity(fragment_count);
    for (fragment_index, chunk) in payload.chunks(max_fragment_payload).enumerate() {
        fragments.push(
            QuicMediaFragment {
                payload_type,
                codec,
                profile_id,
                frame_id,
                timestamp_us,
                flags,
                fragment_index: fragment_index as u16,
                fragment_count: fragment_count as u16,
                payload: Bytes::copy_from_slice(chunk),
            }
            .encode(),
        );
    }
    if fragments.is_empty() {
        fragments.push(
            QuicMediaFragment {
                payload_type,
                codec,
                profile_id,
                frame_id,
                timestamp_us,
                flags,
                fragment_index: 0,
                fragment_count: 1,
                payload: Bytes::new(),
            }
            .encode(),
        );
    }
    Ok(fragments)
}

#[derive(Debug, Default)]
pub struct QuicMediaReassembler {
    config: QuicAuReassemblerConfig,
    stats: QuicAuReassemblerStats,
    pending: HashMap<u32, PendingMediaFrame>,
}

#[derive(Debug)]
struct PendingMediaFrame {
    payload_type: QuicMediaPayloadType,
    codec: QuicMediaCodec,
    profile_id: u32,
    timestamp_us: u64,
    flags: u8,
    fragment_count: u16,
    created_at: Instant,
    updated_at: Instant,
    fragments: BTreeMap<u16, Bytes>,
}

impl QuicMediaReassembler {
    pub fn new(config: QuicAuReassemblerConfig) -> Self {
        Self {
            config,
            stats: QuicAuReassemblerStats::default(),
            pending: HashMap::new(),
        }
    }

    pub fn stats(&self) -> QuicAuReassemblerStats {
        let mut stats = self.stats;
        stats.pending_frames = self.pending.len() as u64;
        stats
    }

    pub fn prune_expired(&mut self) {
        let now = Instant::now();
        self.prune_expired_at(now);
        self.enforce_pending_limit();
    }

    pub fn push_datagram(
        &mut self,
        datagram: &[u8],
    ) -> Result<Option<QuicMediaFrame>, QuinnTransportError> {
        let now = Instant::now();
        self.prune_expired_at(now);
        let fragment = match QuicMediaFragment::decode(datagram) {
            Ok(fragment) => fragment,
            Err(error) => {
                self.stats.rejected_fragments = self.stats.rejected_fragments.saturating_add(1);
                return Err(error);
            }
        };
        {
            let entry =
                self.pending
                    .entry(fragment.frame_id)
                    .or_insert_with(|| PendingMediaFrame {
                        payload_type: fragment.payload_type,
                        codec: fragment.codec,
                        profile_id: fragment.profile_id,
                        timestamp_us: fragment.timestamp_us,
                        flags: fragment.flags,
                        fragment_count: fragment.fragment_count,
                        created_at: now,
                        updated_at: now,
                        fragments: BTreeMap::new(),
                    });

            if entry.payload_type != fragment.payload_type
                || entry.codec != fragment.codec
                || entry.profile_id != fragment.profile_id
                || entry.timestamp_us != fragment.timestamp_us
                || entry.flags != fragment.flags
                || entry.fragment_count != fragment.fragment_count
            {
                self.pending.remove(&fragment.frame_id);
                self.stats.rejected_fragments = self.stats.rejected_fragments.saturating_add(1);
                return Err(QuinnTransportError::Message(
                    "inconsistent media v3 fragment metadata for frame".into(),
                ));
            }

            entry.updated_at = now;
            if entry.fragments.contains_key(&fragment.fragment_index) {
                self.stats.duplicate_fragments = self.stats.duplicate_fragments.saturating_add(1);
                return Ok(None);
            }
            entry
                .fragments
                .insert(fragment.fragment_index, fragment.payload);
        }
        self.enforce_pending_limit();

        if self.pending[&fragment.frame_id].fragments.len()
            != self.pending[&fragment.frame_id].fragment_count as usize
        {
            return Ok(None);
        }

        let completed = self
            .pending
            .remove(&fragment.frame_id)
            .expect("pending media frame exists");
        let total_len = completed
            .fragments
            .values()
            .map(|chunk| chunk.len())
            .sum::<usize>();
        let mut payload = BytesMut::with_capacity(total_len);
        for index in 0..completed.fragment_count {
            let chunk = completed.fragments.get(&index).ok_or_else(|| {
                QuinnTransportError::Message("missing media v3 fragment during reassembly".into())
            })?;
            payload.extend_from_slice(chunk);
        }
        self.stats.completed_frames = self.stats.completed_frames.saturating_add(1);
        Ok(Some(QuicMediaFrame {
            payload_type: completed.payload_type,
            codec: completed.codec,
            profile_id: completed.profile_id,
            frame_id: fragment.frame_id,
            timestamp_us: completed.timestamp_us,
            flags: completed.flags,
            payload: payload.freeze(),
        }))
    }

    fn prune_expired_at(&mut self, now: Instant) {
        let timeout = self.config.frame_timeout;
        if timeout.is_zero() {
            return;
        }
        let mut expired = Vec::new();
        for (frame_id, pending) in &self.pending {
            if now.saturating_duration_since(pending.updated_at) >= timeout {
                expired.push(*frame_id);
            }
        }
        for frame_id in expired {
            if self.pending.remove(&frame_id).is_some() {
                self.stats.expired_frames = self.stats.expired_frames.saturating_add(1);
            }
        }
    }

    fn enforce_pending_limit(&mut self) {
        while self.pending.len() > self.config.max_pending_frames.max(1) {
            let Some(oldest_frame_id) = self
                .pending
                .iter()
                .min_by_key(|(_, pending)| pending.created_at)
                .map(|(frame_id, _)| *frame_id)
            else {
                break;
            };
            if self.pending.remove(&oldest_frame_id).is_some() {
                self.stats.evicted_frames = self.stats.evicted_frames.saturating_add(1);
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct QuicAuReassembler {
    config: QuicAuReassemblerConfig,
    stats: QuicAuReassemblerStats,
    pending: HashMap<u32, PendingFrame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuicAuReassemblerConfig {
    pub frame_timeout: Duration,
    pub max_pending_frames: usize,
}

impl Default for QuicAuReassemblerConfig {
    fn default() -> Self {
        Self {
            frame_timeout: Duration::from_millis(250),
            max_pending_frames: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuicAuReassemblerStats {
    pub completed_frames: u64,
    pub expired_frames: u64,
    pub evicted_frames: u64,
    pub duplicate_fragments: u64,
    pub rejected_fragments: u64,
    pub pending_frames: u64,
}

#[derive(Debug)]
struct PendingFrame {
    timestamp_us: u64,
    is_keyframe: bool,
    fragment_count: u16,
    created_at: Instant,
    updated_at: Instant,
    fragments: BTreeMap<u16, Bytes>,
}

impl QuicAuReassembler {
    pub fn new(config: QuicAuReassemblerConfig) -> Self {
        Self {
            config,
            stats: QuicAuReassemblerStats::default(),
            pending: HashMap::new(),
        }
    }

    pub fn stats(&self) -> QuicAuReassemblerStats {
        let mut stats = self.stats;
        stats.pending_frames = self.pending.len() as u64;
        stats
    }

    pub fn prune_expired(&mut self) {
        let now = Instant::now();
        self.prune_expired_at(now);
        self.enforce_pending_limit();
    }

    pub fn push_datagram(
        &mut self,
        datagram: &[u8],
    ) -> Result<Option<QuicAuFrame>, QuinnTransportError> {
        let now = Instant::now();
        self.prune_expired_at(now);
        let fragment = QuicAuFragment::decode(datagram)?;
        {
            let entry = self
                .pending
                .entry(fragment.frame_id)
                .or_insert_with(|| PendingFrame {
                    timestamp_us: fragment.timestamp_us,
                    is_keyframe: fragment.is_keyframe,
                    fragment_count: fragment.fragment_count,
                    created_at: now,
                    updated_at: now,
                    fragments: BTreeMap::new(),
                });

            if entry.timestamp_us != fragment.timestamp_us
                || entry.is_keyframe != fragment.is_keyframe
                || entry.fragment_count != fragment.fragment_count
            {
                self.pending.remove(&fragment.frame_id);
                self.stats.rejected_fragments = self.stats.rejected_fragments.saturating_add(1);
                return Err(QuinnTransportError::Message(
                    "inconsistent fragment metadata for frame".into(),
                ));
            }

            entry.updated_at = now;
            if entry.fragments.contains_key(&fragment.fragment_index) {
                self.stats.duplicate_fragments = self.stats.duplicate_fragments.saturating_add(1);
                return Ok(None);
            }
            entry
                .fragments
                .insert(fragment.fragment_index, fragment.payload);
        }
        self.enforce_pending_limit();

        if self.pending[&fragment.frame_id].fragments.len()
            != self.pending[&fragment.frame_id].fragment_count as usize
        {
            return Ok(None);
        }

        let completed = self
            .pending
            .remove(&fragment.frame_id)
            .expect("pending frame exists");
        let total_len = completed
            .fragments
            .values()
            .map(|chunk| chunk.len())
            .sum::<usize>();
        let mut payload = BytesMut::with_capacity(total_len);
        for index in 0..completed.fragment_count {
            let chunk = completed.fragments.get(&index).ok_or_else(|| {
                QuinnTransportError::Message("missing fragment during reassembly".into())
            })?;
            payload.extend_from_slice(chunk);
        }
        self.stats.completed_frames = self.stats.completed_frames.saturating_add(1);
        Ok(Some(QuicAuFrame {
            frame_id: fragment.frame_id,
            timestamp_us: completed.timestamp_us,
            is_keyframe: completed.is_keyframe,
            payload: payload.freeze(),
        }))
    }

    fn prune_expired_at(&mut self, now: Instant) {
        let timeout = self.config.frame_timeout;
        if timeout.is_zero() {
            return;
        }
        let mut expired = Vec::new();
        for (frame_id, pending) in &self.pending {
            if now.saturating_duration_since(pending.updated_at) >= timeout {
                expired.push(*frame_id);
            }
        }
        for frame_id in expired {
            if self.pending.remove(&frame_id).is_some() {
                self.stats.expired_frames = self.stats.expired_frames.saturating_add(1);
            }
        }
    }

    fn enforce_pending_limit(&mut self) {
        while self.pending.len() > self.config.max_pending_frames.max(1) {
            let Some(oldest_frame_id) = self
                .pending
                .iter()
                .min_by_key(|(_, pending)| pending.created_at)
                .map(|(frame_id, _)| *frame_id)
            else {
                break;
            };
            if self.pending.remove(&oldest_frame_id).is_some() {
                self.stats.evicted_frames = self.stats.evicted_frames.saturating_add(1);
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum QuinnTransportError {
    #[error("{0}")]
    Message(String),
}
