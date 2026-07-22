use realtime_server::{
    ws::{build_router, RealtimeAppState, ServerRuntimeConfig},
    RealtimeCore, RejectAllBackendTokens,
};
use std::sync::Arc;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let config = ServerRuntimeConfig::from_env()?;
    let core = RealtimeCore::new(config.core.clone(), Arc::new(RejectAllBackendTokens))?;
    warn!(
        "backend token verifier adapter is not configured; registration fails closed until injected"
    );
    let state = RealtimeAppState::new(core, config.clone());
    let _pruner = state.spawn_pruner();
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    info!(bind_addr = %config.bind_addr, "realtime-server listening");
    axum::serve(listener, app).await?;
    Ok(())
}
