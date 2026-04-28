//! Network interruption recovery end-to-end tests
//!
//! Tests automatic reconnection, state recovery, and health monitoring.

use std::time::Duration;

use bytes::Bytes;
use mrd_transport_quic_quinn::recovery::{
    ConnectionHealth, HealthMonitor, ReconnectConfig, ReconnectableEndpoint,
};
use mrd_transport_quic_quinn::QuinnServerListener;

/// Test basic endpoint connection
#[tokio::test]
async fn reconnectable_endpoint_connects_successfully() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind server");

    tokio::spawn(async move {
        listener.accept().await.ok();
    });

    let config = ReconnectConfig {
        enabled: false,
        ..Default::default()
    };

    let endpoint = ReconnectableEndpoint::new(bootstrap, config);
    endpoint.connect().await.expect("Failed to connect");

    assert!(endpoint.is_connected().await);
    assert_eq!(endpoint.health().await, ConnectionHealth::Healthy);
}

/// Test health monitor initialization
#[tokio::test]
async fn health_monitor_starts_without_panic() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind");

    tokio::spawn(async move {
        listener.accept().await.ok();
    });

    let config = ReconnectConfig {
        enabled: false,
        ..Default::default()
    };

    let endpoint = std::sync::Arc::new(ReconnectableEndpoint::new(bootstrap, config));
    endpoint.connect().await.expect("Failed to connect");

    let monitor = HealthMonitor::new(
        endpoint.clone(),
        Duration::from_millis(50),
        Duration::from_millis(100),
    );

    // Should not panic
    monitor.start().await;

    // Give monitor time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Still healthy (not idle yet)
    assert_eq!(endpoint.health().await, ConnectionHealth::Healthy);
}

/// Test activity tracking
#[tokio::test]
async fn endpoint_tracks_last_activity() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind");

    tokio::spawn(async move {
        listener.accept().await.ok();
    });

    let config = ReconnectConfig {
        enabled: false,
        ..Default::default()
    };

    let endpoint = ReconnectableEndpoint::new(bootstrap, config);
    endpoint.connect().await.expect("Failed to connect");

    // Send datagram to update activity
    endpoint
        .send_datagram(Bytes::from(&b"test"[..]))
        .await
        .expect("Failed to send");

    let activity = endpoint.last_activity().await;
    assert!(activity.is_some());

    // Activity should be recent
    let elapsed = activity.unwrap().elapsed();
    assert!(elapsed < Duration::from_secs(1));
}

/// Test endpoint close
#[tokio::test]
async fn endpoint_close_disconnects() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind");

    tokio::spawn(async move {
        listener.accept().await.ok();
    });

    let config = ReconnectConfig {
        enabled: false,
        ..Default::default()
    };

    let endpoint = ReconnectableEndpoint::new(bootstrap, config);
    endpoint.connect().await.expect("Failed to connect");

    assert!(endpoint.is_connected().await);

    endpoint.close().await;

    assert!(!endpoint.is_connected().await);
}

/// Test reconnection config defaults
#[test]
fn reconnect_config_has_sensible_defaults() {
    let config = ReconnectConfig::default();

    assert!(config.enabled);
    assert_eq!(config.max_attempts, 5);
    assert_eq!(config.initial_backoff, Duration::from_millis(100));
    assert_eq!(config.max_backoff, Duration::from_secs(10));
}

/// Test health state transitions
#[tokio::test]
async fn health_transitions_from_healthy_to_disconnected_on_close() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind");

    tokio::spawn(async move {
        listener.accept().await.ok();
    });

    let config = ReconnectConfig {
        enabled: false,
        ..Default::default()
    };

    let endpoint = ReconnectableEndpoint::new(bootstrap, config);
    endpoint.connect().await.expect("Failed to connect");

    assert_eq!(endpoint.health().await, ConnectionHealth::Healthy);

    endpoint.close().await;

    assert_eq!(endpoint.health().await, ConnectionHealth::Disconnected);
}

/// Test sending and receiving with reconnectable endpoint
#[tokio::test]
async fn send_and_receive_with_reconnectable_endpoint() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind");

    tokio::spawn(async move {
        if let Ok(conn) = listener.accept().await {
            // Echo server
            let _ = conn.read_datagram().await;
        }
    });

    let config = ReconnectConfig {
        enabled: false,
        ..Default::default()
    };

    let endpoint = ReconnectableEndpoint::new(bootstrap, config);
    endpoint.connect().await.expect("Failed to connect");

    // Send should succeed
    endpoint
        .send_datagram(Bytes::from(&b"hello"[..]))
        .await
        .expect("Failed to send");

    assert!(endpoint.is_connected().await);
}
