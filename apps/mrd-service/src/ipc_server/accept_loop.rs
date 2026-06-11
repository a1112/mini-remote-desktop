use super::IpcServer;
use mrd_ipc::transport;
use std::time::Duration;

#[cfg(windows)]
use std::sync::Arc;

#[cfg(windows)]
const WINDOWS_IPC_ACCEPT_BACKLOG: usize = 32;

impl IpcServer {
    /// Run the IPC server (accepts connections in a loop)
    pub async fn run(&self) -> anyhow::Result<()> {
        #[cfg(windows)]
        let server =
            Arc::new(transport::IpcServer::bind_with_endpoint(self.endpoint.clone()).await?);

        #[cfg(not(windows))]
        let server = transport::IpcServer::bind_with_endpoint(self.endpoint.clone()).await?;

        tracing::info!("IPC server listening");

        #[cfg(windows)]
        {
            let mut workers = tokio::task::JoinSet::new();
            for _ in 0..WINDOWS_IPC_ACCEPT_BACKLOG {
                let pipe_server = server.clone();
                let connection_server = self.clone();
                workers.spawn(async move {
                    loop {
                        match pipe_server.accept().await {
                            Ok(stream) => {
                                if let Err(e) = connection_server.handle_connection(stream).await {
                                    eprintln!("IPC connection error: {}", e);
                                }
                            }
                            Err(e) => {
                                eprintln!("IPC accept error: {}", e);
                                tokio::time::sleep(accept_retry_delay()).await;
                            }
                        }
                    }
                });
            }

            while let Some(result) = workers.join_next().await {
                if let Err(e) = result {
                    eprintln!("IPC accept worker stopped: {}", e);
                }
            }

            Ok(())
        }

        #[cfg(not(windows))]
        {
            let app_state = self.app_state.clone();
            let ui_launcher = self.ui_launcher.clone();
            loop {
                match server.accept().await {
                    Ok(stream) => {
                        let server_clone = IpcServer {
                            app_state: app_state.clone(),
                            endpoint: self.endpoint.clone(),
                            ui_launcher: ui_launcher.clone(),
                            autostart: crate::shell::default_autostart("mrd-service"),
                        };
                        tokio::spawn(async move {
                            if let Err(e) = server_clone.handle_connection(stream).await {
                                eprintln!("IPC connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("IPC accept error: {}", e);
                        tokio::time::sleep(accept_retry_delay()).await;
                    }
                }
            }
        }
    }
}

fn accept_retry_delay() -> Duration {
    Duration::from_secs(1)
}

#[cfg(test)]
mod tests {
    use super::accept_retry_delay;
    use std::time::Duration;

    #[test]
    fn accept_loop_uses_stable_short_retry_delay() {
        assert_eq!(accept_retry_delay(), Duration::from_secs(1));
    }
}
