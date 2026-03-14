use bytes::Bytes;
use std::time::Duration;

use mrd_transport_quic_quinn::{
    fragment_access_unit, QuicAuReassembler, QuicAuReassemblerConfig, QuinnDatagramPair,
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
