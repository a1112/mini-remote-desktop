use std::{net::{IpAddr, Ipv4Addr, SocketAddr}, sync::Arc};

use bytes::Bytes;
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig};
use rustls::RootCertStore;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuicTransportMetadata {
    pub transport: &'static str,
    pub local_addr: SocketAddr,
    pub peer_addr: SocketAddr,
}

#[derive(Debug, Clone)]
pub struct QuinnDatagramEndpoint {
    endpoint: Endpoint,
    connection: Connection,
    metadata: QuicTransportMetadata,
}

impl QuinnDatagramEndpoint {
    pub fn metadata(&self) -> &QuicTransportMetadata {
        &self.metadata
    }

    pub fn max_datagram_size(&self) -> Option<usize> {
        self.connection.max_datagram_size()
    }

    pub fn send_datagram(&self, payload: Bytes) -> Result<(), QuinnTransportError> {
        self.connection
            .send_datagram(payload)
            .map_err(|error| QuinnTransportError::Message(format!("send_datagram failed: {error}")))
    }

    pub async fn read_datagram(&self) -> Result<Bytes, QuinnTransportError> {
        self.connection
            .read_datagram()
            .await
            .map_err(|error| QuinnTransportError::Message(format!("read_datagram failed: {error}")))
    }
}

impl Drop for QuinnDatagramEndpoint {
    fn drop(&mut self) {
        self.connection.close(0_u32.into(), b"shutdown");
        self.endpoint.close(0_u32.into(), b"shutdown");
    }
}

pub struct QuinnDatagramPair {
    pub client: QuinnDatagramEndpoint,
    pub server: QuinnDatagramEndpoint,
}

impl QuinnDatagramPair {
    pub async fn loopback() -> Result<Self, QuinnTransportError> {
        let server_crypto = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .map_err(|error| QuinnTransportError::Message(format!("generate cert failed: {error}")))?;
        let server_cert = rustls::pki_types::CertificateDer::from(server_crypto.cert);
        let server_key =
            rustls::pki_types::PrivatePkcs8KeyDer::from(server_crypto.signing_key.serialize_der());
        let server_config = ServerConfig::with_single_cert(
            vec![server_cert.clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(server_key),
        )
        .map_err(|error| QuinnTransportError::Message(format!("server config failed: {error}")))?;

        let server_endpoint = Endpoint::server(
            server_config,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        )
        .map_err(|error| QuinnTransportError::Message(format!("server endpoint failed: {error}")))?;
        let server_addr = server_endpoint
            .local_addr()
            .map_err(|error| QuinnTransportError::Message(format!("server local_addr failed: {error}")))?;

        let mut roots = RootCertStore::empty();
        roots
            .add(server_cert)
            .map_err(|error| QuinnTransportError::Message(format!("add root cert failed: {error}")))?;
        let client_config = ClientConfig::with_root_certificates(Arc::new(roots))
            .map_err(|error| QuinnTransportError::Message(format!("client config failed: {error}")))?;

        let mut client_endpoint =
            Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
                .map_err(|error| {
                    QuinnTransportError::Message(format!("client endpoint failed: {error}"))
                })?;
        client_endpoint.set_default_client_config(client_config);

        let client_connecting = client_endpoint
            .connect(server_addr, "localhost")
            .map_err(|error| QuinnTransportError::Message(format!("connect failed: {error}")))?;
        let server_connecting = server_endpoint
            .accept()
            .await
            .ok_or_else(|| QuinnTransportError::Message("server accept returned None".into()))?;

        let (client_connection, server_connection) =
            tokio::join!(client_connecting, server_connecting);
        let client_connection = client_connection.map_err(|error| {
            QuinnTransportError::Message(format!("client handshake failed: {error}"))
        })?;
        let server_connection = server_connection.map_err(|error| {
            QuinnTransportError::Message(format!("server handshake failed: {error}"))
        })?;

        let client_metadata = QuicTransportMetadata {
            transport: "quic_quinn",
            local_addr: client_endpoint.local_addr().map_err(|error| {
                QuinnTransportError::Message(format!("client local_addr failed: {error}"))
            })?,
            peer_addr: client_connection.remote_address(),
        };
        let server_metadata = QuicTransportMetadata {
            transport: "quic_quinn",
            local_addr: server_endpoint.local_addr().map_err(|error| {
                QuinnTransportError::Message(format!("server local_addr failed: {error}"))
            })?,
            peer_addr: server_connection.remote_address(),
        };

        Ok(Self {
            client: QuinnDatagramEndpoint {
                endpoint: client_endpoint,
                connection: client_connection,
                metadata: client_metadata,
            },
            server: QuinnDatagramEndpoint {
                endpoint: server_endpoint,
                connection: server_connection,
                metadata: server_metadata,
            },
        })
    }
}

#[derive(Debug, Error)]
pub enum QuinnTransportError {
    #[error("{0}")]
    Message(String),
}
