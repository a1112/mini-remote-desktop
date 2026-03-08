use bytes::Bytes;
use mrd_transport_quic_quinn::QuinnDatagramPair;

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
    let payload = pair.server.read_datagram().await.expect("read server datagram");

    assert_eq!(payload, Bytes::from_static(b"hello-quic"));
}
