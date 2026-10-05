use super::IpcServer;
use mrd_ipc::transport;
use std::io::ErrorKind;

impl IpcServer {
    /// Handle a single connection
    pub async fn handle_connection(&self, mut stream: transport::IpcStream) -> anyhow::Result<()> {
        #[cfg(windows)]
        if self.product_only {
            return self.handle_product_connection(stream).await;
        }
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

    #[cfg(windows)]
    async fn handle_product_connection(
        &self,
        mut stream: transport::IpcStream,
    ) -> anyhow::Result<()> {
        use super::product::{caller_denied, VerifiedProductCaller};
        use std::time::Duration;
        let mut first_frame = true;
        loop {
            // Unverified clients cannot occupy all accept workers indefinitely.
            let budget = if first_frame {
                Duration::from_secs(3)
            } else {
                Duration::from_secs(60)
            };
            let mut request =
                match tokio::time::timeout(budget, stream.recv_product_request()).await {
                    Ok(Ok(request)) => request,
                    _ => break,
                };
            // Identity is observed after every read because pipe impersonation
            // belongs to the last message. This function reverts before any await.
            let caller = match VerifiedProductCaller::inspect(&stream) {
                Ok(caller) => caller,
                Err(_) => {
                    let _ = stream.send_response(&caller_denied()).await;
                    break;
                }
            };
            first_frame = false;
            let response = match caller.normalize(&mut request) {
                Ok(()) => caller.bind(self).handle_request(request).await,
                Err(denial) => denial,
            };
            // caller pins both process identity and installed files through dispatch.
            if stream.send_response(&response).await.is_err() {
                break;
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
