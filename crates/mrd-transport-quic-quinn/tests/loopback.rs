use bytes::Bytes;
use std::time::Duration;

use mrd_transport_quic_quinn::{
    fragment_access_unit, fragment_media_payload_v3, is_quic_media_v3_datagram, QuicAuReassembler,
    QuicAuReassemblerConfig, QuicMediaCodec, QuicMediaPayloadType, QuicMediaReassembler,
    QuinnDatagramEndpoint, QuinnDatagramPair, QuinnServerListener,
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
