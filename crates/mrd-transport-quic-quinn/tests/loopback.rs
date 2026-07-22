use bytes::Bytes;
use std::time::Duration;

use mrd_transport_quic_quinn::{
    certificate_fingerprint_sha256, fragment_access_unit, fragment_media_payload_v3,
    is_quic_media_v3_datagram, QuicAuReassembler, QuicAuReassemblerConfig, QuicMediaCodec,
    QuicMediaPayloadType, QuicMediaReassembler, QuinnDatagramEndpoint, QuinnDatagramPair,
    QuinnPreparedServer, QuinnReliableLane, QuinnServerListener,
};

#[tokio::test]
async fn quinn_loopback_pair_initializes_and_exposes_metadata() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");

    assert_eq!(pair.client.metadata().transport, "quic_quinn");
    assert_eq!(pair.server.metadata().transport, "quic_quinn");
    assert!(pair.client.max_datagram_size().is_some());
    assert!(pair.server.max_datagram_size().is_some());
}

#[tokio::test]
async fn quinn_loopback_pair_roundtrips_single_datagram() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");

    pair.client
        .send_datagram(Bytes::from_static(b"hello-quic"))
        .expect("send client datagram");
    let payload = pair
        .server
        .read_datagram()
        .await
        .expect("read server datagram");

    assert_eq!(payload, Bytes::from_static(b"hello-quic"));
}

#[tokio::test]
async fn quinn_loopback_pair_roundtrips_reliable_message() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");
    let payload = Bytes::from(vec![0x5a; 256 * 1024]);

    pair.client
        .send_reliable_message(payload.clone())
        .await
        .expect("send reliable client message");
    let received = pair
        .server
        .read_reliable_message(512 * 1024)
        .await
        .expect("read reliable server message");

    assert_eq!(received, payload);
}

#[tokio::test]
async fn quinn_loopback_pair_roundtrips_persistent_reliable_messages() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");
    let first = Bytes::from(vec![0x11; 64 * 1024]);
    let second = Bytes::from(vec![0x22; 96 * 1024]);

    pair.client
        .send_reliable_message_persistent(first.clone())
        .await
        .expect("send first persistent message");
    pair.client
        .send_reliable_message_persistent(second.clone())
        .await
        .expect("send second persistent message");

    let received_first = pair
        .server
        .read_reliable_message_persistent(128 * 1024)
        .await
        .expect("read first persistent message");
    let received_second = pair
        .server
        .read_reliable_message_persistent(128 * 1024)
        .await
        .expect("read second persistent message");

    assert_eq!(received_first, first);
    assert_eq!(received_second, second);
}

#[tokio::test]
async fn quinn_reliable_lane_preserves_control_order() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");

    for sequence in 0_u8..8 {
        pair.client
            .send_reliable_lane_message(
                QuinnReliableLane::Control,
                Bytes::from(vec![sequence; 1024]),
            )
            .await
            .expect("send ordered control message");
    }
    for sequence in 0_u8..8 {
        let received = pair
            .server
            .read_reliable_lane_message(QuinnReliableLane::Control, 2048)
            .await
            .expect("read ordered control message");
        assert_eq!(received, Bytes::from(vec![sequence; 1024]));
    }
}

#[tokio::test]
async fn stalled_bulk_stream_does_not_block_reliable_control() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");
    let bulk_sender = pair.client.clone();
    let blocked_bulk = tokio::spawn(async move {
        bulk_sender
            .send_reliable_lane_message(
                QuinnReliableLane::Bulk,
                Bytes::from(vec![0x5a; 16 * 1024 * 1024]),
            )
            .await
    });
    tokio::task::yield_now().await;

    pair.client
        .send_reliable_lane_message(
            QuinnReliableLane::Control,
            Bytes::from_static(b"interactive-control"),
        )
        .await
        .expect("bulk pressure must not block control send");
    let received = tokio::time::timeout(
        Duration::from_secs(2),
        pair.server
            .read_reliable_lane_message(QuinnReliableLane::Control, 1024),
    )
    .await
    .expect("bulk pressure blocked control receive")
    .expect("read control while bulk is stalled");
    assert_eq!(received, Bytes::from_static(b"interactive-control"));

    blocked_bulk.abort();
}

#[tokio::test]
async fn quinn_loopback_pair_roundtrips_fragmented_access_unit() {
    let pair = QuinnDatagramPair::loopback()
        .await
        .expect("initialize quinn loopback pair");
    let max_datagram_size = pair
        .client
        .max_datagram_size()
        .expect("quinn max datagram size");
    let payload = vec![0x7a; max_datagram_size * 2 + 333];
    let datagrams = fragment_access_unit(7, 123_456, true, &payload, max_datagram_size)
        .expect("fragment large payload");
    assert!(datagrams.len() > 1);

    for datagram in &datagrams {
        pair.client
            .send_datagram(datagram.clone())
            .expect("send fragmented datagram");
    }

    let mut reassembler = QuicAuReassembler::default();
    let mut reassembled = None;
    for _ in 0..datagrams.len() {
        let datagram = pair.server.read_datagram().await.expect("read datagram");
        reassembled = reassembler
            .push_datagram(&datagram)
            .expect("reassemble datagram")
            .or(reassembled);
    }

    let frame = reassembled.expect("completed reassembly");
    assert_eq!(frame.frame_id, 7);
    assert_eq!(frame.timestamp_us, 123_456);
    assert!(frame.is_keyframe);
    assert_eq!(frame.payload, Bytes::from(payload));
}

#[test]
fn media_v3_reassembly_rejects_a_frame_over_its_byte_budget() {
    let fragments = fragment_media_payload_v3(
        QuicMediaPayloadType::AccessUnit,
        QuicMediaCodec::H264,
        0,
        99,
        1,
        false,
        &[0x5a; 16],
        mrd_transport_quic_quinn::QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN + 8,
    )
    .expect("fragment bounded frame");
    let mut reassembler =
        QuicMediaReassembler::new(QuicAuReassemblerConfig::default()).with_max_frame_bytes(12);

    assert!(reassembler
        .push_datagram(&fragments[0])
        .expect("first fragment fits")
        .is_none());
    let error = reassembler
        .push_datagram(&fragments[1])
        .expect_err("completed frame exceeds the configured byte budget");
    assert!(error.to_string().contains("reassembly byte limit"));
    assert_eq!(reassembler.stats().pending_frames, 0);
}

#[test]
fn media_v3_reassembly_evicts_old_frames_at_total_byte_budget() {
    let fragment_size = mrd_transport_quic_quinn::QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN + 8;
    let first = fragment_media_payload_v3(
        QuicMediaPayloadType::AccessUnit,
        QuicMediaCodec::H264,
        0,
        1,
        1,
        false,
        &[0x11; 16],
        fragment_size,
    )
    .expect("fragment first bounded frame");
    let second = fragment_media_payload_v3(
        QuicMediaPayloadType::AccessUnit,
        QuicMediaCodec::H264,
        0,
        2,
        2,
        false,
        &[0x22; 16],
        fragment_size,
    )
    .expect("fragment second bounded frame");
    let mut reassembler = QuicMediaReassembler::new(QuicAuReassemblerConfig::default())
        .with_max_frame_bytes(16)
        .with_max_total_bytes(12);

    assert!(reassembler
        .push_datagram(&first[0])
        .expect("retain first incomplete frame")
        .is_none());
    assert!(reassembler
        .push_datagram(&second[0])
        .expect("retain newer incomplete frame")
        .is_none());

    let stats = reassembler.stats();
    assert_eq!(stats.pending_frames, 1);
    assert_eq!(stats.pending_bytes, 8);
    assert_eq!(stats.evicted_frames, 1);
}

#[test]
fn legacy_reassembly_evicts_old_frames_at_total_byte_budget() {
    let fragment_size = mrd_transport_quic_quinn::QUIC_AU_FRAGMENT_HEADER_LEN + 8;
    let first = fragment_access_unit(1, 1, false, &[0x11; 16], fragment_size)
        .expect("fragment first legacy frame");
    let second = fragment_access_unit(2, 2, false, &[0x22; 16], fragment_size)
        .expect("fragment second legacy frame");
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig::default())
        .with_max_frame_bytes(16)
        .with_max_total_bytes(12);

    assert!(reassembler
        .push_datagram(&first[0])
        .expect("retain first legacy frame")
        .is_none());
    assert!(reassembler
        .push_datagram(&second[0])
        .expect("retain newer legacy frame")
        .is_none());

    let stats = reassembler.stats();
    assert_eq!(stats.pending_frames, 1);
    assert_eq!(stats.pending_bytes, 8);
    assert_eq!(stats.evicted_frames, 1);
}

#[tokio::test]
async fn quinn_reassembler_expires_incomplete_frames() {
    let mut reassembler = QuicAuReassembler::new(QuicAuReassemblerConfig {
        frame_timeout: Duration::from_millis(5),
        max_pending_frames: 4,
    });
    let datagrams = fragment_access_unit(9, 42, false, &[0x55; 4096], 1200)
        .expect("fragment payload for expiry");

    reassembler
        .push_datagram(&datagrams[0])
        .expect("accept first fragment");
    tokio::time::sleep(Duration::from_millis(10)).await;
    reassembler.prune_expired();

    let stats = reassembler.stats();
    assert_eq!(stats.pending_frames, 0);
    assert_eq!(stats.expired_frames, 1);
}

#[tokio::test]
async fn quinn_reassembler_tracks_duplicate_fragments() {
    let mut reassembler = QuicAuReassembler::default();
    let datagrams = fragment_access_unit(11, 77, false, &[0x33; 2048], 1200)
        .expect("fragment payload for duplicates");

    reassembler
        .push_datagram(&datagrams[0])
        .expect("accept first fragment");
    let completed = reassembler
        .push_datagram(&datagrams[0])
        .expect("accept duplicate fragment");

    assert!(completed.is_none());
    assert_eq!(reassembler.stats().duplicate_fragments, 1);
}

#[test]
fn quic_media_v3_reassembles_typed_h264_access_unit() {
    let payload = vec![0x65; 4096];
    let datagrams = fragment_media_payload_v3(
        QuicMediaPayloadType::AccessUnit,
        QuicMediaCodec::H264,
        3,
        77,
        123_456,
        true,
        &payload,
        512,
    )
    .expect("fragment v3 media payload");

    assert!(datagrams.len() > 1);
    assert!(is_quic_media_v3_datagram(&datagrams[0]));

    let mut reassembler = QuicMediaReassembler::default();
    let mut completed = None;
    for datagram in datagrams.iter().rev() {
        completed = reassembler
            .push_datagram(datagram)
            .expect("reassemble v3 media payload")
            .or(completed);
    }

    let frame = completed.expect("completed v3 media frame");
    assert_eq!(frame.payload_type, QuicMediaPayloadType::AccessUnit);
    assert_eq!(frame.codec, QuicMediaCodec::H264);
    assert_eq!(frame.profile_id, 3);
    assert_eq!(frame.frame_id, 77);
    assert_eq!(frame.timestamp_us, 123_456);
    assert!(frame.is_keyframe());
    assert_eq!(frame.payload, Bytes::from(payload));
}

#[test]
fn quic_media_v3_rejects_invalid_magic() {
    let datagrams = fragment_media_payload_v3(
        QuicMediaPayloadType::AccessUnit,
        QuicMediaCodec::H264,
        1,
        10,
        20,
        false,
        b"payload",
        1200,
    )
    .expect("fragment v3 media payload");
    let mut corrupted = datagrams[0].to_vec();
    corrupted[0] = b'X';
    assert!(!is_quic_media_v3_datagram(&corrupted));

    let mut reassembler = QuicMediaReassembler::default();
    let error = reassembler
        .push_datagram(&corrupted)
        .expect_err("invalid magic should fail");

    assert!(error.to_string().contains("invalid media v3 magic"));
}

#[tokio::test]
async fn quinn_bootstrap_supports_explicit_client_server_connection() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("bind explicit quinn server");
    let server_task = tokio::spawn(async move { listener.accept().await });
    let client_endpoint = QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &bootstrap)
        .await
        .expect("connect explicit quinn client");
    let server_endpoint = server_task
        .await
        .expect("join server task")
        .expect("accept server connection");

    client_endpoint
        .send_datagram(Bytes::from_static(b"client-to-server"))
        .expect("send client datagram");
    let server_payload = server_endpoint
        .read_datagram()
        .await
        .expect("read server datagram");
    assert_eq!(server_payload, Bytes::from_static(b"client-to-server"));

    server_endpoint
        .send_datagram(Bytes::from_static(b"server-to-client"))
        .expect("send server datagram");
    let client_payload = client_endpoint
        .read_datagram()
        .await
        .expect("read client datagram");
    assert_eq!(client_payload, Bytes::from_static(b"server-to-client"));

    assert_eq!(bootstrap.transport, "quic_quinn");
    assert_eq!(bootstrap.server_name, "localhost");
    assert!(!bootstrap.cert_der.is_empty());
}

#[tokio::test]
async fn quinn_bootstrap_exposes_a_unique_sha256_certificate_fingerprint() {
    let (_first_listener, first) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("first QUIC listener");
    let (_second_listener, second) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("second QUIC listener");

    let first_fingerprint = certificate_fingerprint_sha256(&first.cert_der);
    let second_fingerprint = certificate_fingerprint_sha256(&second.cert_der);

    assert_eq!(first_fingerprint, first.certificate_fingerprint_sha256());
    assert_eq!(second_fingerprint, second.certificate_fingerprint_sha256());
    assert_ne!(first_fingerprint, second_fingerprint);
}

#[tokio::test]
async fn prepared_quinn_server_preserves_its_certificate_when_bound() {
    let prepared = QuinnPreparedServer::generate().expect("prepare QUIC server material");
    let prepared_cert = prepared.certificate_der().to_vec();
    let prepared_fingerprint = prepared.certificate_fingerprint_sha256();

    let (listener, bootstrap) = prepared
        .bind("127.0.0.1:0")
        .await
        .expect("bind prepared QUIC server");

    assert_eq!(bootstrap.cert_der, prepared_cert);
    assert_eq!(
        bootstrap.certificate_fingerprint_sha256(),
        prepared_fingerprint
    );

    let server_task = tokio::spawn(async move { listener.accept().await });
    let _client = QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &bootstrap)
        .await
        .expect("connect using prepared certificate");
    server_task
        .await
        .expect("join prepared server")
        .expect("prepared server accepts pinned client");
}

#[tokio::test]
async fn quinn_listener_can_pin_the_authenticated_client_ip() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("QUIC listener");
    let server_task =
        tokio::spawn(async move { listener.accept_from("127.0.0.1".parse().unwrap()).await });
    let client = QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &bootstrap)
        .await
        .expect("authenticated-IP client");
    let server = server_task
        .await
        .expect("join server")
        .expect("accept expected client IP");

    assert_eq!(client.metadata().peer_addr.ip(), bootstrap.listen_addr.ip());
    assert_eq!(server.metadata().peer_addr.ip().to_string(), "127.0.0.1");
}

#[tokio::test]
async fn quinn_listener_rejects_a_client_from_the_wrong_ip() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("QUIC listener");
    let mut server_task = tokio::spawn(async move {
        listener
            .accept_from("192.0.2.1".parse().expect("documentation address"))
            .await
    });
    let client = QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &bootstrap)
        .await
        .expect("wrong-IP client completes the TLS handshake");

    assert!(
        tokio::time::timeout(Duration::from_millis(250), &mut server_task)
            .await
            .is_err(),
        "wrong-IP client must not be returned as the authenticated endpoint"
    );
    tokio::time::timeout(Duration::from_secs(1), client.read_datagram())
        .await
        .expect("wrong-IP connection is closed promptly")
        .expect_err("rejected connection cannot receive media");
    server_task.abort();
}

#[tokio::test]
async fn quinn_listener_survives_a_failed_handshake_before_the_valid_client() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("QUIC listener");
    let (_unrelated_listener, unrelated_bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("unrelated QUIC certificate");
    let mut invalid_bootstrap = bootstrap.clone();
    invalid_bootstrap.cert_der = unrelated_bootstrap.cert_der;
    let server_task =
        tokio::spawn(async move { listener.accept_from("127.0.0.1".parse().unwrap()).await });

    QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &invalid_bootstrap)
        .await
        .expect_err("client rejects the unrelated server certificate");
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        !server_task.is_finished(),
        "one failed handshake must not terminate the listener"
    );

    let client = QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &bootstrap)
        .await
        .expect("valid client connects after failed handshake");
    let server = server_task
        .await
        .expect("join server")
        .expect("listener accepts valid client");
    assert_eq!(client.metadata().peer_addr, bootstrap.listen_addr);
    assert_eq!(server.metadata().peer_addr.ip().to_string(), "127.0.0.1");
}

#[tokio::test]
async fn quinn_cloned_endpoint_keeps_connection_alive_until_last_drop() {
    let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("bind explicit quinn server");
    let server_task = tokio::spawn(async move { listener.accept().await });
    let client_endpoint = QuinnDatagramEndpoint::connect_client("127.0.0.1:0", &bootstrap)
        .await
        .expect("connect explicit quinn client");
    let server_endpoint = server_task
        .await
        .expect("join server task")
        .expect("accept server connection");

    let retained_client = client_endpoint.clone();
    drop(client_endpoint);

    retained_client
        .send_datagram(Bytes::from_static(b"clone-still-open"))
        .expect("send client datagram after dropping original");
    let server_payload = server_endpoint
        .read_datagram()
        .await
        .expect("read server datagram after dropping original");

    assert_eq!(server_payload, Bytes::from_static(b"clone-still-open"));
}
