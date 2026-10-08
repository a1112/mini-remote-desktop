// Cross-platform IPC transport
//
// Windows: Named Pipes
// Unix: Unix Domain Sockets

use anyhow::Result;
use serde::Serialize;

/// Default Windows named pipe used by `mrd-service`.
pub const SERVICE_PIPE_NAME: &str = r"\\.\pipe\mrd-service";
#[cfg(unix)]
/// Socket basename inside the current user's private runtime directory.
pub const SERVICE_SOCKET_PATH: &str = "service.sock";
/// Environment variable that overrides the service IPC endpoint.
pub const SERVICE_ENDPOINT_ENV: &str = "MRD_SERVICE_IPC_ENDPOINT";
/// Environment variable that overrides only the narrow service management endpoint.
pub const MANAGEMENT_ENDPOINT_ENV: &str = "MRD_SERVICE_MANAGEMENT_IPC_ENDPOINT";
/// Optional explicit endpoint for the authenticated installed Windows UI channel.
pub const PRODUCT_ENDPOINT_ENV: &str = "MRD_SERVICE_PRODUCT_IPC_ENDPOINT";

#[cfg(windows)]
mod windows_management;
#[cfg(windows)]
pub mod windows_product;

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
            Self::UnixSocket(format!(
                "/tmp/mrd-service-{}/{}",
                unsafe { libc::geteuid() },
                SERVICE_SOCKET_PATH
            ))
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

    /// Build a service endpoint from a non-empty environment variable value.
    pub fn from_env_value(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }

        #[cfg(windows)]
        {
            Some(Self::NamedPipe(value.to_string()))
        }

        #[cfg(unix)]
        {
            Some(Self::UnixSocket(value.to_string()))
        }
    }

    /// Resolve the service endpoint from `MRD_SERVICE_IPC_ENDPOINT`, or use the default.
    pub fn service_from_env_or_default() -> Self {
        std::env::var(SERVICE_ENDPOINT_ENV)
            .ok()
            .and_then(|value| Self::from_env_value(&value))
            .unwrap_or_else(Self::default_service)
    }

    /// Dedicated endpoint derived from the core service endpoint.
    pub fn management_for_service(service: Self) -> Self {
        match service {
            #[cfg(windows)]
            Self::NamedPipe(path) => Self::NamedPipe(format!("{path}-management")),
            #[cfg(unix)]
            Self::UnixSocket(path) => {
                let stem = path.strip_suffix(".sock").unwrap_or(&path);
                Self::UnixSocket(format!("{stem}-management.sock"))
            }
        }
    }

    /// Resolve the dedicated service management endpoint.
    pub fn management_from_env_or_default() -> Self {
        std::env::var(MANAGEMENT_ENDPOINT_ENV)
            .ok()
            .and_then(|value| Self::from_env_value(&value))
            .unwrap_or_else(|| Self::management_for_service(Self::service_from_env_or_default()))
    }

    /// Windows installed UI channel; ordinary Unix clients retain private core IPC.
    #[cfg(windows)]
    pub fn product_for_service(service: Self) -> Self {
        match service {
            Self::NamedPipe(path) => Self::NamedPipe(format!("{path}-product")),
        }
    }

    /// Resolve the dedicated installed UI endpoint without changing core overrides.
    #[cfg(windows)]
    pub fn product_from_env_or_default() -> Self {
        std::env::var(PRODUCT_ENDPOINT_ENV)
            .ok()
            .and_then(|value| Self::from_env_value(&value))
            .unwrap_or_else(|| Self::product_for_service(Self::service_from_env_or_default()))
    }

    /// Return the local named pipe endpoint for platform verification/testing.
    #[cfg(windows)]
    pub fn as_windows_pipe_name(&self) -> &str {
        self.pipe_name()
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
) -> Result<zeroize::Zeroizing<Vec<u8>>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes) as usize;

    if len > MAX_MESSAGE_SIZE {
        anyhow::bail!("IPC message too large: {} bytes", len);
    }

    let mut buf = zeroize::Zeroizing::new(vec![0u8; len]);
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

async fn write_json_message<W, T>(writer: &mut W, message: &T) -> Result<()>
where
    W: tokio::io::AsyncWriteExt + std::marker::Unpin,
    T: Serialize,
{
    // Requests can carry ephemeral enrollment or user credentials. Erase their
    // serialized copies on successful writes, errors and async cancellation.
    let mut json = zeroize::Zeroizing::new(Vec::new());
    serde_json::to_writer(&mut *json, message)?;
    write_message(writer, &json).await
}

// Unix server
#[cfg(unix)]
/// Unix domain socket IPC server.
pub struct IpcServer {
    listener: UnixListener,
    owned_socket: (String, u64, u64),
}

#[cfg(unix)]
impl IpcServer {
    /// Bind the default service endpoint.
    pub async fn bind() -> Result<Self> {
        Self::bind_with_endpoint(IpcEndpoint::default_service()).await
    }

    /// Bind a custom Unix domain socket endpoint.
    pub async fn bind_with_endpoint(endpoint: IpcEndpoint) -> Result<Self> {
        Self::bind_private(endpoint).await
    }

    /// Bind a dedicated owner-only management socket without replacing an existing endpoint.
    pub async fn bind_management_with_endpoint(endpoint: IpcEndpoint) -> Result<Self> {
        anyhow::ensure!(
            endpoint != IpcEndpoint::service_from_env_or_default(),
            "Management IPC must use a distinct endpoint"
        );
        Self::bind_private(endpoint).await
    }

    async fn bind_private(endpoint: IpcEndpoint) -> Result<Self> {
        use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
        let path = endpoint.socket_path();
        let socket_path = std::path::Path::new(path);
        anyhow::ensure!(
            socket_path.is_absolute(),
            "IPC socket path must be absolute"
        );
        anyhow::ensure!(
            socket_path.components().all(|part| matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )),
            "IPC socket path contains an invalid component"
        );
        let parent = socket_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("IPC socket has no parent"))?;
        match std::fs::DirBuilder::new().mode(0o700).create(parent) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let directory = std::fs::symlink_metadata(parent)?;
        anyhow::ensure!(
            directory.is_dir()
                && directory.uid() == unsafe { libc::geteuid() }
                && directory.mode() & 0o777 == 0o700,
            "IPC socket parent must be an owned private directory with mode 0700"
        );
        if let Ok(existing) = std::fs::symlink_metadata(path) {
            anyhow::ensure!(
                existing.file_type().is_socket() && existing.uid() == unsafe { libc::geteuid() },
                "IPC endpoint exists and is not an owned socket"
            );
            match UnixStream::connect(path).await {
                Ok(_) => anyhow::bail!("IPC endpoint already has an active listener"),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                    ) => {}
                Err(error) => return Err(error.into()),
            }
            if let Ok(current) = std::fs::symlink_metadata(path) {
                anyhow::ensure!(
                    current.dev() == existing.dev() && current.ino() == existing.ino(),
                    "IPC endpoint changed during stale-socket recovery"
                );
                std::fs::remove_file(path)?;
            }
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let metadata = std::fs::symlink_metadata(path)?;
        let current_directory = std::fs::symlink_metadata(parent)?;
        anyhow::ensure!(
            current_directory.dev() == directory.dev()
                && current_directory.ino() == directory.ino()
                && current_directory.uid() == unsafe { libc::geteuid() }
                && current_directory.mode() & 0o777 == 0o700,
            "IPC private directory changed during binding"
        );
        Ok(Self {
            listener,
            owned_socket: (path.to_owned(), metadata.dev(), metadata.ino()),
        })
    }

    /// Accept a single IPC stream from a client.
    pub async fn accept(&self) -> Result<IpcStream> {
        let socket = self.listener.accept().await?.0;
        anyhow::ensure!(
            socket.peer_cred()?.uid() == unsafe { libc::geteuid() },
            "IPC peer is not the current user"
        );
        Ok(IpcStream { socket })
    }
}

#[cfg(unix)]
impl Drop for IpcServer {
    fn drop(&mut self) {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let (path, device, inode) = &self.owned_socket;
        if let Ok(current) = std::fs::symlink_metadata(path) {
            if current.file_type().is_socket()
                && current.dev() == *device
                && current.ino() == *inode
            {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

// Windows server
#[cfg(windows)]
/// Windows named-pipe IPC server.
pub struct IpcServer {
    endpoint: IpcEndpoint,
    management_sddl: Option<String>,
    first_instance: tokio::sync::Mutex<Option<tokio::net::windows::named_pipe::NamedPipeServer>>,
}

#[cfg(windows)]
impl IpcServer {
    /// Bind the default service endpoint.
    pub async fn bind() -> Result<Self> {
        Self::bind_with_endpoint(IpcEndpoint::default_service()).await
    }

    /// Bind a custom Windows named-pipe endpoint.
    pub async fn bind_with_endpoint(endpoint: IpcEndpoint) -> Result<Self> {
        Ok(Self {
            endpoint,
            management_sddl: None,
            first_instance: tokio::sync::Mutex::new(None),
        })
    }

    /// Reserve a local-only first pipe instance with a narrow interactive-user ACL.
    pub async fn bind_management_with_endpoint(endpoint: IpcEndpoint) -> Result<Self> {
        anyhow::ensure!(
            endpoint != IpcEndpoint::service_from_env_or_default(),
            "Management IPC must use a distinct endpoint"
        );
        windows_management::validate_local_pipe_name(endpoint.pipe_name())?;
        let management_sddl = windows_management::management_sddl_for_current_process()?;
        let first = windows_management::create_pipe(endpoint.pipe_name(), true, &management_sddl)?;
        Ok(Self {
            endpoint,
            management_sddl: Some(management_sddl),
            first_instance: tokio::sync::Mutex::new(Some(first)),
        })
    }

    /// Reserve a separate data-only product endpoint; caller authentication occurs
    /// after reading each frame in the application dispatcher.
    pub async fn bind_product_with_endpoint(endpoint: IpcEndpoint) -> Result<Self> {
        anyhow::ensure!(
            endpoint != IpcEndpoint::management_from_env_or_default(),
            "Product IPC must use a distinct endpoint"
        );
        Self::bind_management_with_endpoint(endpoint).await
    }

    /// Accept a single IPC stream from a client.
    pub async fn accept(&self) -> Result<IpcStream> {
        let server = if let Some(sddl) = &self.management_sddl {
            match self.first_instance.lock().await.take() {
                Some(first) => first,
                None => windows_management::create_pipe(self.endpoint.pipe_name(), false, sddl)?,
            }
        } else {
            ServerOptions::new()
                .first_pipe_instance(false)
                .create(self.endpoint.pipe_name())?
        };
        server.connect().await?;
        Ok(IpcStream::Server(server))
    }
}

// Unix client
#[cfg(unix)]
/// Unix IPC client factory.
pub struct IpcClient;

#[cfg(unix)]
impl IpcClient {
    /// Connect to the default service endpoint.
    pub async fn connect() -> Result<IpcStream> {
        Self::connect_with_endpoint(&IpcEndpoint::default_service()).await
    }

    /// Connect to a custom Unix domain socket endpoint.
    pub async fn connect_with_endpoint(endpoint: &IpcEndpoint) -> Result<IpcStream> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        let path = std::path::Path::new(endpoint.socket_path());
        anyhow::ensure!(
            path.is_absolute()
                && path.components().all(|part| matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )),
            "IPC socket path is invalid"
        );
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("IPC socket has no parent"))?;
        let directory = std::fs::symlink_metadata(parent)?;
        let owner = unsafe { libc::geteuid() };
        anyhow::ensure!(
            directory.is_dir() && directory.uid() == owner && directory.mode() & 0o777 == 0o700,
            "IPC socket parent is not a private owned directory"
        );
        let metadata = std::fs::symlink_metadata(path)?;
        anyhow::ensure!(
            metadata.file_type().is_socket()
                && metadata.uid() == owner
                && metadata.mode() & 0o777 == 0o600,
            "IPC endpoint is not a private owned socket"
        );
        let socket = UnixStream::connect(path).await?;
        anyhow::ensure!(
            socket.peer_cred()?.uid() == owner,
            "IPC server is not the current user"
        );
        let current = std::fs::symlink_metadata(path)?;
        anyhow::ensure!(
            current.dev() == metadata.dev() && current.ino() == metadata.ino(),
            "IPC socket changed while connecting"
        );
        Ok(IpcStream { socket })
    }

    /// Connect to the dedicated local management socket.
    pub async fn connect_management_with_endpoint(endpoint: &IpcEndpoint) -> Result<IpcStream> {
        Self::connect_with_endpoint(endpoint).await
    }
}

// Windows client
#[cfg(windows)]
/// Windows IPC client factory.
pub struct IpcClient;

#[cfg(windows)]
impl IpcClient {
    /// Connect to the default service endpoint.
    pub async fn connect() -> Result<IpcStream> {
        Self::connect_with_endpoint(&IpcEndpoint::default_service()).await
    }

    /// Connect to a custom Windows named-pipe endpoint.
    pub async fn connect_with_endpoint(endpoint: &IpcEndpoint) -> Result<IpcStream> {
        let pipe = ClientOptions::new().open(endpoint.pipe_name())?;
        Ok(IpcStream::Client(pipe))
    }

    /// Connect using data-only rights, without pipe-instance creation permission.
    pub async fn connect_management_with_endpoint(endpoint: &IpcEndpoint) -> Result<IpcStream> {
        Ok(IpcStream::Client(windows_management::connect_client(
            endpoint.pipe_name(),
        )?))
    }

    /// Authenticate the real installed service before any credential or request is sent.
    pub async fn connect_product_with_endpoint(endpoint: &IpcEndpoint) -> Result<IpcStream> {
        let pipe = windows_management::connect_client(endpoint.pipe_name())?;
        let identity = windows_product::verify_pipe_server(&pipe)?;
        Ok(IpcStream::ProductClient(pipe, identity))
    }
}

// Unix stream
#[cfg(unix)]
/// Unix IPC stream.
pub struct IpcStream {
    socket: UnixStream,
}

#[cfg(unix)]
impl IpcStream {
    /// Return the kernel-reported peer process on macOS. UID-only checks are
    /// insufficient for consent because another same-UID process could forge
    /// an approval; callers must bind sensitive decisions to this PID.
    #[cfg(target_os = "macos")]
    pub fn peer_process_id(&self) -> Result<u32> {
        use std::mem::size_of;
        use std::os::unix::io::AsRawFd;
        let fd = self.socket.as_raw_fd();
        let mut peer_pid: libc::pid_t = 0;
        let mut length = size_of::<libc::pid_t>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&mut peer_pid as *mut libc::pid_t).cast(),
                &mut length,
            )
        };
        anyhow::ensure!(
            result == 0 && peer_pid > 0,
            "unable to authenticate the macOS IPC peer process"
        );
        Ok(peer_pid as u32)
    }

    /// Send an IPC request.
    pub async fn send_request(&mut self, request: &crate::IpcRequest) -> Result<()> {
        write_json_message(&mut self.socket, request).await
    }

    /// Receive an IPC response.
    pub async fn recv_response(&mut self) -> Result<crate::IpcResponse> {
        let buf = read_message(&mut self.socket).await?;
        let response: crate::IpcResponse = serde_json::from_slice(&buf)?;
        Ok(response)
    }

    /// Send an IPC response.
    pub async fn send_response(&mut self, response: &crate::IpcResponse) -> Result<()> {
        write_json_message(&mut self.socket, response).await
    }

    /// Receive an IPC request.
    pub async fn recv_request(&mut self) -> Result<crate::IpcRequest> {
        let buf = read_message(&mut self.socket).await?;
        let request: crate::IpcRequest = serde_json::from_slice(&buf)?;
        Ok(request)
    }
}

// Windows stream
#[cfg(windows)]
/// Windows IPC stream backed by a named-pipe client or server handle.
pub enum IpcStream {
    /// Client-side pipe handle.
    Client(NamedPipeClient),
    /// Installed service process and protected image stay pinned until disconnect.
    ProductClient(NamedPipeClient, windows_product::VerifiedInstalledProcess),
    /// Server-side pipe handle.
    Server(tokio::net::windows::named_pipe::NamedPipeServer),
}

#[cfg(windows)]
impl IpcStream {
    /// Product callers have a small frame budget before their identity is known.
    /// Reject the length prefix before allocating or waiting for its payload.
    pub async fn recv_product_request(&mut self) -> Result<crate::IpcRequest> {
        use tokio::io::AsyncReadExt;
        let IpcStream::Server(pipe) = self else {
            anyhow::bail!("Product request inspection requires a server pipe");
        };
        let mut prefix = [0_u8; 4];
        pipe.read_exact(&mut prefix).await?;
        let size = u32::from_le_bytes(prefix) as usize;
        anyhow::ensure!(
            size > 0 && size <= 64 * 1024,
            "Product IPC frame exceeds the request budget"
        );
        let mut payload = zeroize::Zeroizing::new(vec![0_u8; size]);
        pipe.read_exact(&mut payload).await?;
        Ok(serde_json::from_slice(&payload)?)
    }

    /// Send an IPC request.
    pub async fn send_request(&mut self, request: &crate::IpcRequest) -> Result<()> {
        match self {
            IpcStream::Client(pipe) | IpcStream::ProductClient(pipe, _) => {
                write_json_message(pipe, request).await
            }
            IpcStream::Server(pipe) => write_json_message(pipe, request).await,
        }
    }

    /// Receive an IPC response.
    pub async fn recv_response(&mut self) -> Result<crate::IpcResponse> {
        let buf = match self {
            IpcStream::Client(pipe) | IpcStream::ProductClient(pipe, _) => {
                read_message(pipe).await?
            }
            IpcStream::Server(pipe) => read_message(pipe).await?,
        };
        let response: crate::IpcResponse = serde_json::from_slice(&buf)?;
        Ok(response)
    }

    /// Send an IPC response.
    pub async fn send_response(&mut self, response: &crate::IpcResponse) -> Result<()> {
        match self {
            IpcStream::Client(pipe) | IpcStream::ProductClient(pipe, _) => {
                write_json_message(pipe, response).await
            }
            IpcStream::Server(pipe) => write_json_message(pipe, response).await,
        }
    }

    /// Receive an IPC request.
    pub async fn recv_request(&mut self) -> Result<crate::IpcRequest> {
        let buf = match self {
            IpcStream::Client(pipe) | IpcStream::ProductClient(pipe, _) => {
                read_message(pipe).await?
            }
            IpcStream::Server(pipe) => read_message(pipe).await?,
        };
        let request: crate::IpcRequest = serde_json::from_slice(&buf)?;
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IpcRequest;
    #[cfg(unix)]
    use crate::IpcResponse;

    #[test]
    fn frame_format_is_valid() {
        let request = IpcRequest::ListDevices;
        let json = serde_json::to_string(&request).unwrap();
        let bytes = serde_json::to_vec(&request).unwrap();
        let len = bytes.len() as u32;
        assert_eq!(bytes, json.as_bytes());
        assert_eq!(len.to_le_bytes().len(), 4);
    }

    #[test]
    #[cfg(windows)]
    fn endpoint_from_env_value_uses_named_pipe_on_windows() {
        let endpoint = IpcEndpoint::from_env_value(r"\\.\pipe\mrd-service-local-controller")
            .expect("custom endpoint");
        assert_eq!(
            endpoint,
            IpcEndpoint::named_pipe(r"\\.\pipe\mrd-service-local-controller")
        );
    }

    #[test]
    #[cfg(unix)]
    fn endpoint_from_env_value_uses_unix_socket_on_unix() {
        let endpoint =
            IpcEndpoint::from_env_value("/tmp/mrd-service-local-controller.sock").unwrap();
        assert_eq!(
            endpoint,
            IpcEndpoint::unix_socket("/tmp/mrd-service-local-controller.sock")
        );
    }

    #[test]
    fn endpoint_from_env_value_rejects_blank_values() {
        assert!(IpcEndpoint::from_env_value("  ").is_none());
    }

    #[test]
    fn management_endpoint_is_distinct_from_custom_core_endpoint() {
        #[cfg(windows)]
        let (core, expected) = (
            IpcEndpoint::named_pipe(r"\\.\pipe\custom-core"),
            IpcEndpoint::named_pipe(r"\\.\pipe\custom-core-management"),
        );
        #[cfg(unix)]
        let (core, expected) = (
            IpcEndpoint::unix_socket("/tmp/custom-core.sock"),
            IpcEndpoint::unix_socket("/tmp/custom-core-management.sock"),
        );
        assert_eq!(IpcEndpoint::management_for_service(core), expected);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn unix_socket_ipc_roundtrip() -> Result<()> {
        let directory = private_test_directory("roundtrip");
        let endpoint = IpcEndpoint::unix_socket(directory.join("service.sock").to_string_lossy());
        let server = IpcServer::bind_with_endpoint(endpoint.clone()).await?;
        let server_handle = tokio::spawn(async move {
            let mut stream = server.accept().await?;

            let request = stream.recv_request().await?;
            assert!(matches!(request, IpcRequest::ListDevices));

            let response = IpcResponse::DeviceList { devices: vec![] };
            stream.send_response(&response).await?;
            Ok::<(), anyhow::Error>(())
        });

        let mut stream = IpcClient::connect_with_endpoint(&endpoint).await?;
        stream.send_request(&IpcRequest::ListDevices).await?;
        let response = stream.recv_response().await?;
        assert!(matches!(response, IpcResponse::DeviceList { .. }));

        drop(stream);
        server_handle.await??;
        std::fs::remove_dir(directory)?;
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_client_refuses_a_socket_without_private_owner_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let directory = private_test_directory("client-mode");
        let path = directory.join("server.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let endpoint = IpcEndpoint::unix_socket(path.to_string_lossy());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(IpcClient::connect_with_endpoint(&endpoint).await.is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let client = IpcClient::connect_with_endpoint(&endpoint).await.unwrap();
        let peer = listener.accept().await.unwrap();
        drop(client);
        drop(peer);
        drop(listener);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_core_socket_never_displaces_an_active_listener_or_regular_file() {
        let directory = private_test_directory("core-security");
        let path = directory.join("core.sock");
        std::fs::write(&path, "preserve").unwrap();
        let endpoint = IpcEndpoint::unix_socket(path.to_string_lossy());
        assert!(IpcServer::bind_with_endpoint(endpoint.clone())
            .await
            .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "preserve");
        std::fs::remove_file(&path).unwrap();
        let active = IpcServer::bind_with_endpoint(endpoint.clone())
            .await
            .unwrap();
        assert!(IpcServer::bind_with_endpoint(endpoint).await.is_err());
        drop(active);
        assert!(!path.exists());
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_core_socket_is_private_and_rejects_a_shared_parent() {
        use std::os::unix::fs::PermissionsExt;
        let directory = private_test_directory("core-mode");
        let path = directory.join("core.sock");
        let endpoint = IpcEndpoint::unix_socket(path.to_string_lossy());
        let server = IpcServer::bind_with_endpoint(endpoint.clone())
            .await
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(server);
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(IpcServer::bind_with_endpoint(endpoint).await.is_err());
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    fn private_test_directory(name: &str) -> std::path::PathBuf {
        use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
        // macOS temp_dir() uses a long /var/folders path. mkdtemp atomically
        // creates a fresh 0700 directory under short /tmp without env overrides.
        let mut template = std::ffi::CString::new(format!("/tmp/mrd-{name}-XXXXXX"))
            .unwrap()
            .into_bytes_with_nul();
        let created = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
        assert!(
            !created.is_null(),
            "private IPC fixture directory creation failed: {}",
            std::io::Error::last_os_error()
        );
        let path = std::path::PathBuf::from(
            unsafe { std::ffi::CStr::from_ptr(created) }
                .to_str()
                .unwrap(),
        );
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o777, 0o700);
        assert!(
            path.join("management.sock-link")
                .as_os_str()
                .as_bytes()
                .len()
                < 104
        );
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_management_socket_can_restart_and_recover_an_owned_stale_socket() {
        use std::os::unix::fs::PermissionsExt;
        let directory = private_test_directory("management-restart");
        let path = directory
            .join("management.sock")
            .to_string_lossy()
            .into_owned();
        let endpoint = IpcEndpoint::unix_socket(&path);
        let first = IpcServer::bind_management_with_endpoint(endpoint.clone())
            .await
            .unwrap();
        assert!(IpcServer::bind_management_with_endpoint(endpoint.clone())
            .await
            .is_err());
        drop(first);
        assert!(!std::path::Path::new(&path).exists());
        let second = IpcServer::bind_management_with_endpoint(endpoint.clone())
            .await
            .unwrap();
        drop(second);
        let stale = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(stale);
        let recovered = IpcServer::bind_management_with_endpoint(endpoint)
            .await
            .unwrap();
        drop(recovered);
        assert!(!std::path::Path::new(&path).exists());
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_management_socket_never_replaces_regular_files_or_symlinks() {
        let directory = private_test_directory("management-files");
        let path = directory
            .join("management.sock")
            .to_string_lossy()
            .into_owned();
        std::fs::write(&path, "preserve").unwrap();
        assert!(
            IpcServer::bind_management_with_endpoint(IpcEndpoint::unix_socket(&path))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "preserve");
        let link = format!("{path}-link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(
            IpcServer::bind_management_with_endpoint(IpcEndpoint::unix_socket(&link))
                .await
                .is_err()
        );
        std::fs::remove_file(link).unwrap();
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
