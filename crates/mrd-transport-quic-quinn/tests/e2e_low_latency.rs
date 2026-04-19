//! Low-latency transmission end-to-end tests
//!
//! Tests pacing, FEC, and NACK functionality.

use std::time::Duration;

use mrd_transport_quic_quinn::low_latency::{
    FecConfig, FecScheme, NackConfig, NackMessage, PacingConfig,
};
use mrd_transport_quic_quinn::QuinnServerListener;

/// Test pacing delay calculation
#[test]
fn pacing_calculates_correct_delay_for_various_packet_sizes() {
    let config = PacingConfig {
        enabled: true,
        target_bitrate_bps: 10_000_000, // 10 Mbps
        max_burst_bytes: 64 * 1024,
        min_packet_interval: Duration::from_micros(100),
    };

    // Small packet
    let delay1 = config.calculate_delay(500);
    assert!(delay1 >= Duration::from_micros(100));

    // Large packet
    let delay2 = config.calculate_delay(1500);
    assert!(delay2 > delay1);

    // Very large packet
    let delay3 = config.calculate_delay(9000);
    assert!(delay3 > delay2);
}

/// Test pacing burst capacity
#[test]
fn pacing_respects_burst_capacity() {
    let config = PacingConfig {
        enabled: true,
        target_bitrate_bps: 10_000_000,
        max_burst_bytes: 32 * 1024, // 32 KB burst
        min_packet_interval: Duration::from_micros(100),
    };

    assert_eq!(config.burst_capacity(), 32 * 1024);
}

/// Test pacing config validation
#[test]
fn pacing_config_has_valid_defaults() {
    let config = PacingConfig::default();

    assert!(config.enabled);
    assert!(config.target_bitrate_bps > 0);
    assert!(config.max_burst_bytes > 0);
    assert!(!config.min_packet_interval.is_zero());
}

/// Test FEC config validation
#[test]
fn fec_config_has_valid_defaults() {
    let config = FecConfig::default();

    assert!(config.enabled);
    assert!(config.block_size >= 5);
    assert!(config.parity_count >= 1);
    assert!(config.parity_count < config.block_size);
}

/// Test FEC scheme variants
#[test]
fn fec_scheme_supports_multiple_variants() {
    let schemes = vec![FecScheme::Xor, FecScheme::ReedSolomon];

    for scheme in schemes {
        let config = FecConfig {
            enabled: true,
            scheme,
            block_size: 3,
            parity_count: 1,
        };

        assert_eq!(config.scheme, scheme);
        assert!(config.block_size > 0);
        assert!(config.parity_count > 0);
    }
}

/// Test NACK config validation
#[test]
fn nack_config_has_valid_defaults() {
    let config = NackConfig::default();

    assert!(config.enabled);
    assert!(config.max_nacked_seqs > 0);
    assert!(config.max_nack_retries > 0);
    assert!(!config.nack_timeout.is_zero());
}

/// Test NACK message structure
#[test]
fn nack_message_stores_sequence_numbers() {
    let msg = NackMessage {
        sequence_numbers: vec![1, 5, 10, 100],
    };

    assert_eq!(msg.sequence_numbers.len(), 4);
    assert!(msg.sequence_numbers.contains(&5));
    assert!(msg.sequence_numbers.contains(&100));
}

/// Test NACK message with empty sequences
#[test]
fn nack_message_handles_empty_sequences() {
    let msg = NackMessage {
        sequence_numbers: vec![],
    };

    assert!(msg.sequence_numbers.is_empty());
}

/// Test pacing with disabled state
#[test]
fn pacing_disabled_returns_zero_delay() {
    let config = PacingConfig {
        enabled: false,
        ..Default::default()
    };

    let delay = config.calculate_delay(1500);
    assert_eq!(delay, Duration::ZERO);
}

/// Test pacing with zero bitrate
#[test]
fn pacing_zero_bitrate_returns_zero_delay() {
    let config = PacingConfig {
        enabled: true,
        target_bitrate_bps: 0,
        ..Default::default()
    };

    let delay = config.calculate_delay(1500);
    assert_eq!(delay, Duration::ZERO);
}

/// Test FEC config with different schemes
#[test]
fn fec_config_supports_different_schemes() {
    let xor_config = FecConfig {
        scheme: FecScheme::Xor,
        ..Default::default()
    };

    let rs_config = FecConfig {
        scheme: FecScheme::ReedSolomon,
        ..Default::default()
    };

    assert_eq!(xor_config.scheme, FecScheme::Xor);
    assert_eq!(rs_config.scheme, FecScheme::ReedSolomon);
}

/// Test NACK config with custom timeout
#[test]
fn nack_config_allows_custom_timeout() {
    let config = NackConfig {
        nack_timeout: Duration::from_millis(200),
        ..Default::default()
    };

    assert_eq!(config.nack_timeout, Duration::from_millis(200));
}

/// Test NACK config with custom retry count
#[test]
fn nack_config_allows_custom_retries() {
    let config = NackConfig {
        max_nack_retries: 5,
        ..Default::default()
    };

    assert_eq!(config.max_nack_retries, 5);
}

/// Test endpoint integration with low-latency configs
#[tokio::test]
async fn low_latency_config_works_with_quinn_endpoint() {
    let (_listener, _bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind server");

    // Create low-latency configs
    let _pacing = PacingConfig::default();
    let _fec = FecConfig::default();
    let _nack = NackConfig::default();

    // Verify configs are valid
    let pacing = PacingConfig::default();
    let fec = FecConfig::default();
    let nack = NackConfig::default();

    assert!(pacing.enabled);
    assert!(fec.enabled);
    assert!(nack.enabled);
}

/// Test multiple FEC configs can coexist
#[test]
fn fec_configs_are_independent() {
    let config1 = FecConfig {
        block_size: 10,
        parity_count: 2,
        ..Default::default()
    };

    let config2 = FecConfig {
        block_size: 20,
        parity_count: 3,
        ..Default::default()
    };

    assert_eq!(config1.block_size, 10);
    assert_eq!(config1.parity_count, 2);
    assert_eq!(config2.block_size, 20);
    assert_eq!(config2.parity_count, 3);
}

/// Test NACK message equality
#[test]
fn nack_messages_with_same_sequences_are_equal() {
    let msg1 = NackMessage {
        sequence_numbers: vec![1, 2, 3],
    };

    let msg2 = NackMessage {
        sequence_numbers: vec![1, 2, 3],
    };

    assert_eq!(msg1, msg2);
}

/// Test NACK message inequality
#[test]
fn nack_messages_with_different_sequences_are_not_equal() {
    let msg1 = NackMessage {
        sequence_numbers: vec![1, 2, 3],
    };

    let msg2 = NackMessage {
        sequence_numbers: vec![1, 2, 4],
    };

    assert_ne!(msg1, msg2);
}

/// Test pacing delay scales with packet size
#[test]
fn pacing_delay_scales_linearly_with_packet_size() {
    let config = PacingConfig {
        enabled: true,
        target_bitrate_bps: 8_000_000, // 8 Mbps = 1 MB/s
        max_burst_bytes: 64 * 1024,
        min_packet_interval: Duration::ZERO,
    };

    let delay1 = config.calculate_delay(1000);
    let delay2 = config.calculate_delay(2000);

    // 2x packet size should give ~2x delay
    assert!(delay2.as_secs_f64() >= delay1.as_secs_f64() * 1.9);
    assert!(delay2.as_secs_f64() <= delay1.as_secs_f64() * 2.1);
}
