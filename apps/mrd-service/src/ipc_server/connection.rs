use super::IpcServer;
use mrd_ipc::transport;
use std::io::ErrorKind;

impl IpcServer {
    /// Handle a single connection
    pub async fn handle_connection(&self, mut stream: transport::IpcStream) -> anyhow::Result<()> {
        loop {
            match stream.recv_request().await {
                Ok(request) => {
                    let shutdown_mode = match &request {
                        mrd_ipc::IpcRequest::ShutdownService { mode } => Some(mode.clone()),
                        _ => None,
                    };
                    let response = self.handle_request(request).await;
                    let sent = stream.send_response(&response).await;
                    if matches!(response, mrd_ipc::IpcResponse::Ack) {
                        if let Some(mode) = shutdown_mode {
                            // A disappeared requester must not strand accepted shutdown.
                            self.app_state.shutdown.acknowledge(mode);
                        }
                    }
                    if let Err(e) = sent {
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
