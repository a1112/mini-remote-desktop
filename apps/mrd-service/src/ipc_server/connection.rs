use super::IpcServer;
use mrd_ipc::transport;
use std::io::ErrorKind;

impl IpcServer {
    /// Handle a single connection
    pub async fn handle_connection(&self, mut stream: transport::IpcStream) -> anyhow::Result<()> {
        loop {
            match stream.recv_request().await {
                Ok(request) => {
                    let response = self.handle_request(request).await;
                    if let Err(e) = stream.send_response(&response).await {
                        eprintln!("Failed to send IPC response: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    if !is_connection_closed_error(&e) {
                        eprintln!("IPC request error: {}", e);
                    }
                    break;
                }
            }
        }
        Ok(())
    }
}

pub(super) fn is_connection_closed_error(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<std::io::Error>() {
        Some(io_error) => matches!(
            io_error.kind(),
            ErrorKind::UnexpectedEof
                | ErrorKind::BrokenPipe
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
        ),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn connection_module_classifies_expected_closed_stream_errors() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "closed pipe",
        ));

        assert!(super::is_connection_closed_error(&error));
    }
}
