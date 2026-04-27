// Cross-platform IPC transport
//
// Windows: Named Pipes
// Unix: Unix Domain Sockets

use anyhow::Result;
use serde_json;

pub const SERVICE_PIPE_NAME: &str = r"\\.\pipe\mrd-service";
#[cfg(unix)]
pub const SERVICE_SOCKET_PATH: &str = "/tmp/mrd-service.sock";

const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// IPC endpoint used by clients and servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcEndpoint {
    /// Windows named pipe endpoint.
    #[cfg(windows)]
    NamedPipe(String),
    /// Unix domain socket endpoint.
    #[cfg(unix)]
    UnixSocket(String),
}

impl IpcEndpoint {
    /// Default service endpoint used by production Rdesk and mrd-service.
    pub fn default_service() -> Self {
        #[cfg(windows)]
        {
            Self::NamedPipe(SERVICE_PIPE_NAME.to_string())
        }

        #[cfg(unix)]
        {
            Self::UnixSocket(SERVICE_SOCKET_PATH.to_string())
        }
    }

    /// Construct a Windows named pipe endpoint.
    #[cfg(windows)]
    pub fn named_pipe(path: impl Into<String>) -> Self {
        Self::NamedPipe(path.into())
    }

    /// Construct a Unix domain socket endpoint.
    #[cfg(unix)]
    pub fn unix_socket(path: impl Into<String>) -> Self {
        Self::UnixSocket(path.into())
    }

    #[cfg(windows)]
    fn pipe_name(&self) -> &str {
        match self {
            Self::NamedPipe(path) => path,
        }
    }

    #[cfg(unix)]
    fn socket_path(&self) -> &str {
        match self {
            Self::UnixSocket(path) => path,
        }
    }
}

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, ServerOptions};

async fn read_message<R: tokio::io::AsyncReadExt + std::marker::Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;

    if len > MAX_MESSAGE_SIZE {
        anyhow::bail!("IPC message too large: {} bytes", len);
    }

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_message<W: tokio::io::AsyncWriteExt + std::marker::Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<()> {
    let len = data.len() as u32;
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

// Unix server
#[cfg(unix)]
pub struct IpcServer {
    listener: UnixListener,
}

#[cfg(unix)]
impl IpcServer {
    pub async fn bind() -> Result<Self> {
        Self::bind_with_endpoint(IpcEndpoint::default_service()).await
    }

    pub async fn bind_with_endpoint(endpoint: IpcEndpoint) -> Result<Self> {
        let socket_path = endpoint.socket_path();
        let _ = std::fs::remove_file(socket_path);
        let listener = UnixListener::bind(socket_path)?;
        Ok(Self { listener })
    }

    pub async fn accept(&self) -> Result<IpcStream> {
        let socket = self.listener.accept().await?.0;
        Ok(IpcStream { socket })
    }
}

// Windows server
#[cfg(windows)]
pub struct IpcServer {
    endpoint: IpcEndpoint,
}

#[cfg(windows)]
impl IpcServer {
    pub async fn bind() -> Result<Self> {
        Self::bind_with_endpoint(IpcEndpoint::default_service()).await
    }

    pub async fn bind_with_endpoint(endpoint: IpcEndpoint) -> Result<Self> {
        Ok(Self { endpoint })
    }

    pub async fn accept(&self) -> Result<IpcStream> {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(self.endpoint.pipe_name())?;
        server.connect().await?;
        Ok(IpcStream::Server(server))
    }
}

// Unix client
#[cfg(unix)]
pub struct IpcClient;

#[cfg(unix)]
impl IpcClient {
    pub async fn connect() -> Result<IpcStream> {
        Self::connect_with_endpoint(&IpcEndpoint::default_service()).await
    }

    pub async fn connect_with_endpoint(endpoint: &IpcEndpoint) -> Result<IpcStream> {
        let socket = UnixStream::connect(endpoint.socket_path()).await?;
        Ok(IpcStream { socket })
    }
}

// Windows client
#[cfg(windows)]
pub struct IpcClient;

#[cfg(windows)]
impl IpcClient {
    pub async fn connect() -> Result<IpcStream> {
        Self::connect_with_endpoint(&IpcEndpoint::default_service()).await
    }

    pub async fn connect_with_endpoint(endpoint: &IpcEndpoint) -> Result<IpcStream> {
        let pipe = ClientOptions::new().open(endpoint.pipe_name())?;
        Ok(IpcStream::Client(pipe))
    }
}

// Unix stream
#[cfg(unix)]
pub struct IpcStream {
    socket: UnixStream,
}

#[cfg(unix)]
impl IpcStream {
    pub async fn send_request(&mut self, request: &crate::IpcRequest) -> Result<()> {
        let json = serde_json::to_string(request)?;
        write_message(&mut self.socket, json.as_bytes()).await
    }

    pub async fn recv_response(&mut self) -> Result<crate::IpcResponse> {
        let buf = read_message(&mut self.socket).await?;
        let response: crate::IpcResponse = serde_json::from_slice(&buf)?;
        Ok(response)
    }

    pub async fn send_response(&mut self, response: &crate::IpcResponse) -> Result<()> {
        let json = serde_json::to_string(response)?;
        write_message(&mut self.socket, json.as_bytes()).await
    }

    pub async fn recv_request(&mut self) -> Result<crate::IpcRequest> {
        let buf = read_message(&mut self.socket).await?;
        let request: crate::IpcRequest = serde_json::from_slice(&buf)?;
        Ok(request)
    }
}

// Windows stream
#[cfg(windows)]
pub enum IpcStream {
    Client(NamedPipeClient),
    Server(tokio::net::windows::named_pipe::NamedPipeServer),
}

#[cfg(windows)]
impl IpcStream {
    pub async fn send_request(&mut self, request: &crate::IpcRequest) -> Result<()> {
        let json = serde_json::to_string(request)?;
        match self {
            IpcStream::Client(pipe) => write_message(pipe, json.as_bytes()).await,
            IpcStream::Server(pipe) => write_message(pipe, json.as_bytes()).await,
        }
    }

    pub async fn recv_response(&mut self) -> Result<crate::IpcResponse> {
        let buf = match self {
            IpcStream::Client(pipe) => read_message(pipe).await?,
            IpcStream::Server(pipe) => read_message(pipe).await?,
        };
        let response: crate::IpcResponse = serde_json::from_slice(&buf)?;
        Ok(response)
    }

    pub async fn send_response(&mut self, response: &crate::IpcResponse) -> Result<()> {
        let json = serde_json::to_string(response)?;
        match self {
            IpcStream::Client(pipe) => write_message(pipe, json.as_bytes()).await,
            IpcStream::Server(pipe) => write_message(pipe, json.as_bytes()).await,
        }
    }

    pub async fn recv_request(&mut self) -> Result<crate::IpcRequest> {
        let buf = match self {
            IpcStream::Client(pipe) => read_message(pipe).await?,
            IpcStream::Server(pipe) => read_message(pipe).await?,
        };
        let request: crate::IpcRequest = serde_json::from_slice(&buf)?;
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IpcRequest, IpcResponse};

    #[test]
    fn frame_format_is_valid() {
        let request = IpcRequest::ListDevices;
        let json = serde_json::to_string(&request).unwrap();
        let len = json.len() as u32;
        assert_eq!(len.to_le_bytes().len(), 4);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn unix_socket_ipc_roundtrip() -> Result<()> {
        use tokio::time::{sleep, Duration};

        let server_handle = tokio::spawn(async {
            let server = IpcServer::bind().await?;
            let mut stream = server.accept().await?;

            let request = stream.recv_request().await?;
            assert!(matches!(request, IpcRequest::ListDevices));

            let response = IpcResponse::DeviceList { devices: vec![] };
            stream.send_response(&response).await?;
            Ok::<(), anyhow::Error>(())
        });

        sleep(Duration::from_millis(100)).await;

        let mut stream = IpcClient::connect().await?;
        stream.send_request(&IpcRequest::ListDevices).await?;
        let response = stream.recv_response().await?;
        assert!(matches!(response, IpcResponse::DeviceList { .. }));

        server_handle.await??;
        Ok(())
    }
}
