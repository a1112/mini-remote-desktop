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
        self.run_inner(None).await
    }

    /// Announce startup only after endpoint binding succeeds.
    pub async fn run_with_ready(
        &self,
        ready: tokio::sync::oneshot::Sender<()>,
    ) -> anyhow::Result<()> {
        self.run_inner(Some(ready)).await
    }

    async fn run_inner(
        &self,
        ready: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> anyhow::Result<()> {
        let server = if self.management_only {
            transport::IpcServer::bind_management_with_endpoint(self.endpoint.clone()).await?
        } else {
            transport::IpcServer::bind_with_endpoint(self.endpoint.clone()).await?
        };
        #[cfg(windows)]
        let server = Arc::new(server);

        tracing::info!("IPC server listening");
        if let Some(ready) = ready {
            let _ = ready.send(());
        }

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
            loop {
                match server.accept().await {
                    Ok(stream) => {
                        let server_clone = self.clone();
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
