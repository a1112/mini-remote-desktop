//! QUIC transport end-to-end integration tests
//!
//! Tests QUIC connection establishment, data transfer, fragmentation,
//! reassembly, and error recovery.

use std::time::Duration;

// We'll test the Quinn implementation directly
use bytes::Bytes;
use mrd_transport_quic_quinn::{
    fragment_access_unit, QuicAuFragment, QuicAuReassembler, QuicAuReassemblerConfig,
    QuinnDatagramEndpoint, QuinnDatagramPair, QuinnServerListener, QUIC_AU_FRAGMENT_HEADER_LEN,
};

/// Test basic loopback connection establishment
#[tokio::test]
async fn quic_loopback_connection_established() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("Failed to establish loopback connection");

    let client_metadata = pair.client.metadata();
    let server_metadata = pair.server.metadata();

    assert_eq!(client_metadata.transport, "quic_quinn");
    assert_eq!(server_metadata.transport, "quic_quinn");

    // Verify addresses are cross-linked
    assert_eq!(client_metadata.peer_addr, server_metadata.local_addr);
    assert_eq!(server_metadata.peer_addr, client_metadata.local_addr);
}

/// Test server bootstrap metadata is correct
#[tokio::test]
async fn quic_server_bootstrap_contains_valid_metadata() {
    let (_listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind server");

    assert_eq!(bootstrap.transport, "quic_quinn");
    assert_eq!(bootstrap.server_name, "localhost");
    assert!(!bootstrap.cert_der.is_empty());
    assert!(bootstrap.listen_addr.port() > 0);
}

/// Test client can connect to server with bootstrap
#[tokio::test]
async fn quic_client_connects_with_bootstrap() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind server");

    let server_task = tokio::spawn(async move { listener.accept().await });

    let client = QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &bootstrap)
        .await
        .expect("Failed to connect client");

    let server = server_task
        .await
        .expect("Server task failed")
        .expect("Failed to get server endpoint");

    // Verify bidirectional metadata
    assert_eq!(client.metadata().peer_addr, server.metadata().local_addr);
    assert_eq!(server.metadata().peer_addr, client.metadata().local_addr);
}

/// Test single datagram transmission (client to server)
#[tokio::test]
async fn quic_single_datagram_transmitted_client_to_server() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("Failed to establish loopback connection");

    let test_data = Bytes::from(&b"Hello, QUIC!"[..]);

    // Send from client to server
    pair.client
        .send_datagram(test_data.clone())
        .expect("Failed to send datagram");

    // Receive on server
    let received = pair
        .server
        .read_datagram()
        .await
        .expect("Failed to read datagram");

    assert_eq!(received, test_data);
}

/// Test single datagram transmission (server to client)
#[tokio::test]
async fn quic_single_datagram_transmitted_server_to_client() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("Failed to establish loopback connection");

    let test_data = Bytes::from(&b"Hello from server!"[..]);

    // Send from server to client
    pair.server
        .send_datagram(test_data.clone())
        .expect("Failed to send datagram");

    // Receive on client
    let received = pair
        .client
        .read_datagram()
        .await
        .expect("Failed to read datagram");

    assert_eq!(received, test_data);
}

/// Test multiple sequential datagrams
#[tokio::test]
async fn quic_multiple_sequential_datagrams_transmitted() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("Failed to establish loopback connection");

    let messages = vec![
        Bytes::from("Message 1"),
        Bytes::from("Message 2"),
        Bytes::from("Message 3"),
    ];

    // Send all messages from client to server
    for msg in &messages {
        pair.client
            .send_datagram(msg.clone())
            .expect("Failed to send datagram");
    }

    // Receive all messages on server
    for expected in &messages {
        let received = pair
            .server
            .read_datagram()
            .await
            .expect("Failed to read datagram");
        assert_eq!(&received, expected);
    }
}

/// Test maximum datagram size is reported
#[tokio::test]
async fn quic_max_datagram_size_is_reported() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("Failed to establish loopback connection");

    let client_max = pair.client.max_datagram_size();
    let server_max = pair.server.max_datagram_size();

    // Both sides should report similar max sizes
    assert!(
        client_max.is_some(),
        "Client should report max datagram size"
    );
    assert!(
        server_max.is_some(),
        "Server should report max datagram size"
    );

    let client_max = client_max.unwrap();
    let server_max = server_max.unwrap();

    // Max sizes should be reasonable (> 1000 bytes)
    assert!(
        client_max > 1000,
        "Client max datagram size too small: {}",
        client_max
    );
    assert!(
        server_max > 1000,
        "Server max datagram size too small: {}",
        server_max
    );
}

/// Test access unit fragment encoding and decoding
#[test]
fn quic_au_fragment_encoded_and_decoded_correctly() {
    let fragment = QuicAuFragment {
        frame_id: 12345,
        timestamp_us: 123456789,
        is_keyframe: true,
        fragment_index: 2,
        fragment_count: 5,
        payload: Bytes::from(&b"test payload data"[..]),
    };

    let encoded = fragment.encode();
    let decoded = QuicAuFragment::decode(&encoded[..]).expect("Failed to decode fragment");

    assert_eq!(decoded.frame_id, fragment.frame_id);
    assert_eq!(decoded.timestamp_us, fragment.timestamp_us);
    assert_eq!(decoded.is_keyframe, fragment.is_keyframe);
    assert_eq!(decoded.fragment_index, fragment.fragment_index);
    assert_eq!(decoded.fragment_count, fragment.fragment_count);
    assert_eq!(decoded.payload, fragment.payload);
}

/// Test access unit fragment header length is constant
#[test]
fn quic_au_fragment_header_length_matches_constant() {
    let fragment = QuicAuFragment {
        frame_id: 0,
        timestamp_us: 0,
        is_keyframe: false,
        fragment_index: 0,
        fragment_count: 1,
        payload: Bytes::new(),
    };

    let encoded = fragment.encode();
    // Header includes: frame_id (4) + timestamp_us (8) + is_keyframe (1) +
    //                  fragment_index (2) + fragment_count (2) = 17 bytes
    assert_eq!(encoded.len(), QUIC_AU_FRAGMENT_HEADER_LEN);
}

/// Test access unit fragmentation creates correct fragment count
#[test]
fn quic_access_unit_fragmented_into_correct_number_of_fragments() {
    let payload = vec![0xAB_u8; 2000]; // 2000 bytes
    let max_datagram_size = 512; // Small max size

    let fragments = fragment_access_unit(1, 1000000, false, &payload, max_datagram_size)
        .expect("Failed to fragment access unit");

    // Each fragment can carry (512 - 17) = 495 bytes of payload
    // 2000 / 495 = 4.04 -> 5 fragments
    assert_eq!(fragments.len(), 5);

    // Verify each fragment can be decoded
    for frag in &fragments {
        let decoded = QuicAuFragment::decode(frag).expect("Failed to decode fragment");
        assert_eq!(decoded.fragment_count, 5);
    }
}

/// Test access unit fragmentation with zero payload
#[test]
fn quic_access_unit_with_zero_payload_creates_single_fragment() {
    let payload = vec![0_u8; 0];
    let max_datagram_size = 512;

    let fragments = fragment_access_unit(1, 1000000, true, &payload, max_datagram_size)
        .expect("Failed to fragment access unit");

    assert_eq!(fragments.len(), 1);

    let decoded = QuicAuFragment::decode(&fragments[0]).expect("Failed to decode fragment");
    assert!(decoded.is_keyframe);
    assert_eq!(decoded.fragment_index, 0);
    assert_eq!(decoded.fragment_count, 1);
    assert!(decoded.payload.is_empty());
}

/// Test reassembler correctly reassembles single fragment
#[test]
fn quic_reassembler_handles_single_fragment() {
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default());

    let fragment = QuicAuFragment {
        frame_id: 1,
        timestamp_us: 1000000,
        is_keyframe: true,
        fragment_index: 0,
        fragment_count: 1,
        payload: Bytes::from(&b"single frame data"[..]),
    };

    let encoded = fragment.encode();
    let result = reassembler
        .push_datagram(&encoded[..])
        .expect("Failed to push datagram");

    assert!(result.is_some(), "Should have completed frame");
    let frame = result.unwrap();
    assert_eq!(frame.frame_id, 1);
    assert_eq!(frame.timestamp_us, 1000000);
    assert!(frame.is_keyframe);
    assert_eq!(frame.payload, Bytes::from(&b"single frame data"[..]));
}

/// Test reassembler correctly reassembles multiple fragments
#[test]
fn quic_reassembler_reassembles_multiple_fragments() {
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default());

    let payload = Bytes::from(&b"large payload data that needs fragmentation"[..]);
    let fragments =
        fragment_access_unit(5, 2000000, false, &payload, 100).expect("Failed to fragment");

    let mut result = None;
    for encoded in &fragments {
        result = reassembler
            .push_datagram(&encoded[..])
            .expect("Failed to push datagram");
    }

    assert!(
        result.is_some(),
        "Should have completed frame after all fragments"
    );
    let frame = result.unwrap();
    assert_eq!(frame.frame_id, 5);
    assert_eq!(frame.timestamp_us, 2000000);
    assert!(!frame.is_keyframe);
    assert_eq!(frame.payload, payload);
}

/// Test reassembler handles out-of-order fragments
#[test]
fn quic_reassembler_handles_out_of_order_fragments() {
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default());

    let payload = Bytes::from(&b"data for out of order test"[..]);
    let mut fragments =
        fragment_access_unit(10, 3000000, true, &payload, 50).expect("Failed to fragment");

    // Reverse fragment order
    fragments.reverse();

    let mut result = None;
    for encoded in &fragments {
        result = reassembler
            .push_datagram(&encoded[..])
            .expect("Failed to push datagram");
    }

    assert!(result.is_some(), "Should have completed frame");
    let frame = result.unwrap();
    assert_eq!(frame.payload, payload);
}

/// Test reassembler rejects duplicate fragments
#[test]
fn quic_reassembler_rejects_duplicate_fragments() {
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default());

    // Create a frame with multiple fragments
    let payload = vec![0xAB_u8; 50]; // Small payload
    let fragments =
        fragment_access_unit(20, 4000000, false, &payload, 30).expect("Failed to fragment");

    // Send first fragment
    let encoded_0 = fragments[0].clone();
    reassembler.push_datagram(&encoded_0[..]).ok();

    // Send first fragment again (duplicate) - should be detected
    let result = reassembler.push_datagram(&encoded_0[..]);
    assert!(result.is_ok(), "Duplicate should not error");
    assert!(
        result.unwrap().is_none(),
        "Duplicate should not complete frame"
    );

    let stats = reassembler.stats();
    assert_eq!(
        stats.duplicate_fragments, 1,
        "Duplicate fragment should be counted"
    );

    // Send remaining fragments to complete frame
    for encoded in fragments.iter().skip(1) {
        let _ = reassembler.push_datagram(&encoded[..]);
    }
}

/// Test reassembler expires incomplete frames
#[test]
fn quic_reassembler_expires_incomplete_frames() {
    let config = QuicAuReassemblerConfig {
        frame_timeout: Duration::from_millis(50),
        max_pending_frames: 10,
    };
    let mut reassembler = QuicAuReassembler::new(config);

    let fragment = QuicAuFragment {
        frame_id: 99,
        timestamp_us: 5000000,
        is_keyframe: false,
        fragment_index: 0,
        fragment_count: 3, // Will never complete
        payload: Bytes::from(&b"incomplete"[..]),
    };

    let encoded = fragment.encode();
    reassembler
        .push_datagram(&encoded[..])
        .expect("Failed to push datagram");

    // Frame should be pending
    assert_eq!(reassembler.stats().pending_frames, 1);

    // Wait for expiration
    std::thread::sleep(Duration::from_millis(60));
    reassembler.prune_expired();

    // Frame should be expired
    assert_eq!(reassembler.stats().pending_frames, 0);
    assert_eq!(reassembler.stats().expired_frames, 1);
}

/// Test end-to-end: fragmented frame transmission via QUIC
#[tokio::test]
async fn quic_e2e_fragmented_frame_transmitted() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("Failed to establish loopback connection");

    let max_size = pair.client.max_datagram_size().unwrap_or(1200);

    // Create a large payload that will be fragmented
    let large_payload = vec![0xCD_u8; max_size * 3];

    // Fragment on sender side
    let fragments = fragment_access_unit(100, 6000000, true, &large_payload, max_size)
        .expect("Failed to fragment");

    // Send all fragments
    for fragment in &fragments {
        pair.client
            .send_datagram(fragment.clone())
            .expect("Failed to send fragment");
    }

    // Reassemble on receiver side
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default());
    let mut received_frame = None;

    for _ in 0..fragments.len() {
        let datagram = pair
            .server
            .read_datagram()
            .await
            .expect("Failed to read datagram");
        if let Some(frame) = reassembler
            .push_datagram(&datagram[..])
            .expect("Failed to process datagram")
        {
            received_frame = Some(frame);
            break;
        }
    }

    assert!(
        received_frame.is_some(),
        "Should have received complete frame"
    );
    let frame = received_frame.unwrap();
    assert_eq!(frame.frame_id, 100);
    assert_eq!(frame.timestamp_us, 6000000);
    assert!(frame.is_keyframe);
    assert_eq!(frame.payload.len(), large_payload.len());
    assert_eq!(frame.payload.to_vec(), large_payload);
}

/// Test connection close is graceful
#[tokio::test]
async fn quic_connection_closed_gracefully() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("Failed to establish loopback connection");

    // Send a message successfully
    pair.client
        .send_datagram(Bytes::from(&b"before close"[..]))
        .expect("Failed to send");

    let _ = pair
        .server
        .read_datagram()
        .await
        .expect("Failed to receive");

    // Drop the pair (should close connection gracefully)
    drop(pair);

    // Give time for close to propagate
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Test passes if no panic occurs
}
