use mrd_identity::DeviceIdentity;
use mrd_ipc::MediaProfile;
use mrd_proto::{DeviceId, SessionId};
use mrd_service::{
    handlers::session::start_lan_remote_session,
    lan_discovery::{
        ingest_legacy_lan_announcement, ingest_signed_lan_announcement,
        process_lan_discovery_packet, LanAnnouncement, LanDiscoveryConfig, LanDiscoveryPacket,
        LanMediaBootstrap, LanQuicBootstrap, LanSessionBootstrap, LanSessionRequest,
        SignedLanAnnouncement, SignedLanSessionBootstrap, SignedLanSessionRequest,
        DISCOVERY_APP_ID, DISCOVERY_MAGIC, SIGNED_LAN_PROTOCOL_VERSION,
    },
    AppState, NoOpTray,
};
use mrd_transport_quic_quinn::{certificate_fingerprint_sha256, QuinnServerListener};
use ring::rand::SystemRandom;
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::UdpSocket, time::timeout};

const NOW_MS: u64 = 1_000_000;

fn identity() -> DeviceIdentity {
    DeviceIdentity::generate(&SystemRandom::new()).expect("test identity")
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_millis() as u64
}

fn announcement(
    signer: &DeviceIdentity,
    device_id: &str,
    device_name: &str,
    instance_id: &str,
    discovery_port: u16,
    discovery_endpoint: SocketAddr,
    issued_at_ms: u64,
    expires_at_ms: u64,
    nonce: [u8; 16],
) -> SignedLanAnnouncement {
    SignedLanAnnouncement::sign(
        signer,
        1,
        LanAnnouncement {
            magic: DISCOVERY_MAGIC.to_string(),
            app_id: DISCOVERY_APP_ID.to_string(),
            instance_id: instance_id.to_string(),
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
            device_type: "rdesk".to_string(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            discovery_port,
            transports: vec!["quic".to_string(), "quic_stream_media_v2".to_string()],
            service_build_id: Some("signed-lan-test".to_string()),
            media_protocol_version: Some(3),
            media_capabilities: vec![
                "decode.software".to_string(),
                "quic_stream_media_v2".to_string(),
            ],
            mac_address: None,
            timestamp_ms: issued_at_ms,
        },
        discovery_endpoint,
        expires_at_ms,
        nonce,
    )
    .expect("sign announcement")
}

fn session_request(
    controller: &DeviceIdentity,
    target: &DeviceIdentity,
) -> SignedLanSessionRequest {
    SignedLanSessionRequest::sign(
        controller,
        LanSessionRequest {
            magic: DISCOVERY_MAGIC.to_string(),
            app_id: DISCOVERY_APP_ID.to_string(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            instance_id: "controller-instance".to_string(),
            session_id: "session-1".to_string(),
            source_device_id: "controller-device".to_string(),
            source_device_name: "Controller".to_string(),
            source_key_id: controller.key_id().to_string(),
            source_key_epoch: 1,
            target_device_id: "target-device".to_string(),
            target_key_id: target.key_id().to_string(),
            target_key_epoch: 1,
            transport_kind: "quic".to_string(),
            source_discovery_port: Some(21116),
            source_endpoint: "192.168.1.60:40000".parse().unwrap(),
            source_media_capabilities: vec![
                "decode.software".to_string(),
                "quic_stream_media_v2".to_string(),
            ],
            requested_media_profile: Some(MediaProfile::default()),
            timestamp_ms: NOW_MS,
            expires_at_ms: NOW_MS + 5_000,
            nonce: [3; 16],
        },
    )
    .expect("sign request")
}

#[test]
fn signed_announcement_rejects_device_name_and_endpoint_tampering() {
    let signer = identity();
    let signed = announcement(
        &signer,
        "peer-device",
        "Peer Device",
        "peer-instance",
        21116,
        "192.168.1.50:21116".parse().unwrap(),
        NOW_MS,
        NOW_MS + 10_000,
        [1; 16],
    );

    signed.verify(NOW_MS).expect("valid announcement");

    let mut tampered_device = signed.clone();
    tampered_device.payload.announcement.device_id = "attacker-device".to_string();
    assert!(tampered_device.verify(NOW_MS).is_err());

    let mut tampered_name = signed.clone();
    tampered_name.payload.announcement.device_name = "Attacker".to_string();
    assert!(tampered_name.verify(NOW_MS).is_err());

    let mut tampered_endpoint = signed;
    tampered_endpoint.payload.announcement.discovery_port = 31337;
    assert!(tampered_endpoint.verify(NOW_MS).is_err());

    let mut tampered_address = announcement(
        &signer,
        "peer-device",
        "Peer Device",
        "peer-instance",
        21116,
        "192.168.1.50:21116".parse().unwrap(),
        NOW_MS,
        NOW_MS + 10_000,
        [9; 16],
    );
    tampered_address.payload.discovery_endpoint = "192.168.1.99:21116".parse().unwrap();
    assert!(tampered_address.verify(NOW_MS).is_err());
}

#[test]
fn expired_announcement_is_rejected() {
    let signer = identity();
    let signed = announcement(
        &signer,
        "peer-device",
        "Peer Device",
        "peer-instance",
        21116,
        "192.168.1.50:21116".parse().unwrap(),
        NOW_MS - 10_000,
        NOW_MS - 3_000,
        [2; 16],
    );

    assert!(signed.verify(NOW_MS).is_err());
}

#[tokio::test]
async fn discovery_probe_gets_a_route_bound_signed_unicast_response() {
    let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let service_addr = service_socket.local_addr().unwrap();
    let source_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let config = LanDiscoveryConfig {
        discovery_port: service_addr.port(),
        ..LanDiscoveryConfig::default()
    };
    let app_state = Arc::new(AppState::with_tray_and_lan_discovery_config(
        Arc::new(std::sync::Mutex::new(NoOpTray::new())),
        config,
    ));
    app_state.devices.lock().await.register(
        DeviceId("route-bound-target".to_string()),
        "Route Bound Target".to_string(),
    );
    let probe = LanDiscoveryPacket::Probe {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: "remote-probe-instance".to_string(),
        device_id: None,
        timestamp_ms: current_time_ms(),
    };

    process_lan_discovery_packet(
        &service_socket,
        &app_state,
        &serde_json::to_vec(&probe).unwrap(),
        source_socket.local_addr().unwrap(),
    )
    .await
    .expect("handle discovery probe");

    let mut buffer = [0_u8; 65_535];
    let (len, observed_source) =
        timeout(Duration::from_secs(1), source_socket.recv_from(&mut buffer))
            .await
            .expect("signed response timeout")
            .expect("signed response");
    let packet: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len]).unwrap();
    let LanDiscoveryPacket::SignedAnnounce(signed) = packet else {
        panic!("probe must receive a signed unicast announcement");
    };

    signed
        .verify(current_time_ms())
        .expect("signed route-bound announcement");
    assert_eq!(observed_source, service_addr);
    assert_eq!(signed.payload.discovery_endpoint, observed_source);
}

#[tokio::test]
async fn replayed_announcement_is_rejected_without_replacing_the_peer() {
    let app_state = Arc::new(AppState::new());
    let signer = identity();
    let signed = announcement(
        &signer,
        "peer-device",
        "Peer Device",
        "peer-instance",
        21116,
        "192.168.1.50:21116".parse().unwrap(),
        NOW_MS,
        NOW_MS + 10_000,
        [4; 16],
    );
    let source: SocketAddr = "192.168.1.50:21116".parse().unwrap();

    ingest_signed_lan_announcement(&app_state, signed.clone(), source, NOW_MS)
        .await
        .expect("first observation");
    assert!(
        ingest_signed_lan_announcement(&app_state, signed, source, NOW_MS + 1)
            .await
            .is_err()
    );

    let snapshot = app_state.lan_discovery.snapshot().await;
    assert_eq!(snapshot.peers.len(), 1);
    assert_eq!(snapshot.peers[0].device_name, "Peer Device");
}

#[tokio::test]
async fn signed_announcement_rejects_a_spoofed_udp_source_address() {
    let app_state = Arc::new(AppState::new());
    let signer = identity();
    let signed = announcement(
        &signer,
        "peer-device",
        "Peer Device",
        "peer-instance",
        21116,
        "192.168.1.50:21116".parse().unwrap(),
        NOW_MS,
        NOW_MS + 10_000,
        [10; 16],
    );

    let error = ingest_signed_lan_announcement(
        &app_state,
        signed,
        "192.168.1.99:21116".parse().unwrap(),
        NOW_MS,
    )
    .await
    .expect_err("signed discovery endpoint must match the UDP source address");

    assert!(error.to_string().contains("endpoint"));
    assert!(app_state.lan_discovery.snapshot().await.peers.is_empty());
}

#[tokio::test]
async fn untrusted_signed_peer_is_discoverable_but_not_controllable() {
    let app_state = Arc::new(AppState::new());
    let signer = identity();
    let signed = announcement(
        &signer,
        "untrusted-device",
        "Untrusted Device",
        "untrusted-instance",
        21116,
        "192.168.1.51:21116".parse().unwrap(),
        NOW_MS,
        NOW_MS + 10_000,
        [5; 16],
    );

    ingest_signed_lan_announcement(
        &app_state,
        signed,
        "192.168.1.51:21116".parse().unwrap(),
        NOW_MS,
    )
    .await
    .expect("authenticated discovery remains diagnostic");

    let snapshot = app_state.lan_discovery.snapshot().await;
    assert_eq!(snapshot.peers.len(), 1);
    assert_eq!(snapshot.peers[0].device_id.0, "untrusted-device");
    assert!(!snapshot.peers[0].p2p_available);
    assert!(app_state
        .lan_discovery
        .peer_control_addr(&DeviceId("untrusted-device".to_string()))
        .await
        .is_none());

    let session_id = SessionId("untrusted-session".to_string());
    let response = start_lan_remote_session(
        &app_state,
        session_id.clone(),
        DeviceId("untrusted-device".to_string()),
        "quic".to_string(),
        None,
    )
    .await;
    assert!(matches!(response, mrd_ipc::IpcResponse::Error { .. }));
    assert!(app_state.sessions.lock().await.get(&session_id).is_none());
}

#[test]
fn signed_session_request_is_bound_to_the_expected_target() {
    let controller = identity();
    let target = identity();
    let other_target = identity();
    let signed = session_request(&controller, &target);

    signed
        .verify_for_target(NOW_MS, target.key_id(), 1)
        .expect("request targets the expected peer");
    assert!(signed
        .verify_for_target(NOW_MS, other_target.key_id(), 1)
        .is_err());

    let mut tampered_source = signed.clone();
    tampered_source.payload.source_endpoint = "192.168.1.99:40000".parse().unwrap();
    assert!(tampered_source
        .verify_for_target(NOW_MS, target.key_id(), 1)
        .is_err());

    let mut missing_source = serde_json::to_value(&signed).unwrap();
    missing_source
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
        .unwrap()
        .remove("source_endpoint");
    assert!(serde_json::from_value::<SignedLanSessionRequest>(missing_source).is_err());

    let mut tampered = signed;
    tampered.payload.session_id = "cross-session".to_string();
    assert!(tampered
        .verify_for_target(NOW_MS, target.key_id(), 1)
        .is_err());
}

#[tokio::test]
async fn signed_bootstrap_rejects_quic_certificate_substitution() {
    let controller = identity();
    let target = identity();
    let attacker = identity();
    let request = session_request(&controller, &target);
    let (_listener, quic) = QuinnServerListener::bind("127.0.0.1:0")
        .await
        .expect("QUIC listener");
    let media = LanMediaBootstrap {
        transport_kind: "quic".to_string(),
        quic: Some(LanQuicBootstrap {
            listen_addr: quic.listen_addr.to_string(),
            server_name: quic.server_name,
            certificate_fingerprint_sha256: certificate_fingerprint_sha256(&quic.cert_der),
            cert_der: quic.cert_der,
        }),
    };
    let payload = LanSessionBootstrap {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
        instance_id: "target-instance".to_string(),
        session_id: request.payload.session_id.clone(),
        controller_key_id: controller.key_id().to_string(),
        controller_key_epoch: 1,
        target_key_id: target.key_id().to_string(),
        target_key_epoch: 1,
        request_nonce: request.payload.nonce,
        accepted: true,
        message: Some("accepted".to_string()),
        media: Some(media),
        media_profile: None,
        timestamp_ms: NOW_MS,
        expires_at_ms: NOW_MS + 5_000,
        nonce: [6; 16],
    };
    let signed =
        SignedLanSessionBootstrap::sign(&target, payload.clone()).expect("target signs bootstrap");

    signed
        .verify_for_request(NOW_MS, &request, target.public_key(), 1)
        .expect("valid target bootstrap");

    let mut replaced = signed;
    replaced
        .payload
        .media
        .as_mut()
        .and_then(|media| media.quic.as_mut())
        .expect("QUIC bootstrap")
        .cert_der = vec![0x41; 64];
    assert!(replaced
        .verify_for_request(NOW_MS, &request, target.public_key(), 1)
        .is_err());

    let mut attacker_payload = payload;
    let attacker_quic = attacker_payload
        .media
        .as_mut()
        .and_then(|media| media.quic.as_mut())
        .expect("QUIC bootstrap");
    attacker_quic.cert_der = vec![0x42; 64];
    attacker_quic.certificate_fingerprint_sha256 =
        certificate_fingerprint_sha256(&attacker_quic.cert_der);
    attacker_payload.target_key_id = attacker.key_id().to_string();
    let attacker_signed = SignedLanSessionBootstrap::sign(&attacker, attacker_payload)
        .expect("attacker signs replacement");
    assert!(attacker_signed
        .verify_for_request(NOW_MS, &request, target.public_key(), 1)
        .is_err());
}

#[tokio::test]
async fn unsigned_legacy_peer_is_diagnostic_only_and_cannot_start_a_session() {
    let config = LanDiscoveryConfig {
        allow_unsigned_diagnostics: true,
        ..LanDiscoveryConfig::default()
    };
    let app_state = Arc::new(AppState::with_tray_and_lan_discovery_config(
        Arc::new(std::sync::Mutex::new(NoOpTray::new())),
        config,
    ));
    app_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    let legacy = LanAnnouncement {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: "legacy-instance".to_string(),
        device_id: "legacy-device".to_string(),
        device_name: "Legacy Device".to_string(),
        device_type: "rdesk".to_string(),
        protocol_version: 1,
        discovery_port: 21116,
        transports: vec!["quic".to_string(), "quic_stream_media_v2".to_string()],
        service_build_id: None,
        media_protocol_version: None,
        media_capabilities: vec![
            "decode.software".to_string(),
            "quic_stream_media_v2".to_string(),
        ],
        mac_address: None,
        timestamp_ms: NOW_MS,
    };

    ingest_legacy_lan_announcement(
        &app_state,
        legacy,
        "192.168.1.52:21116".parse().unwrap(),
        NOW_MS,
    )
    .await
    .expect("legacy diagnostic observation");

    let snapshot = app_state.lan_discovery.snapshot().await;
    assert_eq!(snapshot.peers.len(), 1);
    assert!(!snapshot.peers[0].p2p_available);
    assert!(app_state
        .lan_discovery
        .peer_control_addr(&DeviceId("legacy-device".to_string()))
        .await
        .is_none());

    let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let request = LanDiscoveryPacket::RemoteSessionRequest {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: "legacy-controller".to_string(),
        session_id: "legacy-session".to_string(),
        source_device_id: "legacy-controller-device".to_string(),
        source_device_name: "Legacy Controller".to_string(),
        transport_kind: "quic".to_string(),
        source_discovery_port: Some(21116),
        source_media_capabilities: vec![
            "decode.software".to_string(),
            "quic_stream_media_v2".to_string(),
        ],
        requested_media_profile: None,
        timestamp_ms: NOW_MS,
    };
    let bytes = serde_json::to_vec(&request).unwrap();

    process_lan_discovery_packet(
        &service_socket,
        &app_state,
        &bytes,
        source_socket.local_addr().unwrap(),
    )
    .await
    .expect("legacy packet is ignored safely");

    assert!(app_state
        .sessions
        .lock()
        .await
        .get(&SessionId("legacy-session".to_string()))
        .is_none());
    let mut buffer = [0_u8; 1024];
    assert!(timeout(
        Duration::from_millis(50),
        source_socket.recv_from(&mut buffer)
    )
    .await
    .is_err());
}

#[tokio::test]
async fn unsigned_legacy_peer_is_hidden_when_diagnostics_are_not_explicitly_enabled() {
    let app_state = Arc::new(AppState::new());
    let legacy = LanAnnouncement {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id: "legacy-default-off-instance".to_string(),
        device_id: "legacy-default-off-device".to_string(),
        device_name: "Legacy Default Off".to_string(),
        device_type: "rdesk".to_string(),
        protocol_version: 1,
        discovery_port: 21_116,
        transports: vec!["quic".to_string()],
        service_build_id: None,
        media_protocol_version: None,
        media_capabilities: Vec::new(),
        mac_address: None,
        timestamp_ms: NOW_MS,
    };

    let error = ingest_legacy_lan_announcement(
        &app_state,
        legacy,
        "192.168.1.53:21116".parse().unwrap(),
        NOW_MS,
    )
    .await
    .expect_err("legacy discovery must be opt-in");

    assert!(error
        .to_string()
        .contains("unsigned LAN diagnostics are disabled"));
    assert!(app_state.lan_discovery.snapshot().await.peers.is_empty());
}

#[tokio::test]
async fn untrusted_signed_session_request_has_no_session_side_effect_and_sends_no_ack() {
    let app_state = Arc::new(AppState::new());
    app_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    let controller = identity();
    let issued_at_ms = current_time_ms();
    let session_id = SessionId("untrusted-inbound-session".to_string());
    let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let source_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let request = SignedLanSessionRequest::sign(
        &controller,
        LanSessionRequest {
            magic: DISCOVERY_MAGIC.to_string(),
            app_id: DISCOVERY_APP_ID.to_string(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            instance_id: "untrusted-controller-instance".to_string(),
            session_id: session_id.0.clone(),
            source_device_id: "untrusted-controller-device".to_string(),
            source_device_name: "Untrusted Controller".to_string(),
            source_key_id: controller.key_id().to_string(),
            source_key_epoch: 1,
            target_device_id: "target-device".to_string(),
            target_key_id: app_state
                .device_identities
                .machine_key_id()
                .expect("local machine key")
                .to_string(),
            target_key_epoch: app_state
                .device_identities
                .machine_key_epoch()
                .expect("local machine key epoch"),
            transport_kind: "quic".to_string(),
            source_discovery_port: Some(21_116),
            source_endpoint: source_socket.local_addr().unwrap(),
            source_media_capabilities: vec![
                "decode.software".to_string(),
                "quic_stream_media_v2".to_string(),
            ],
            requested_media_profile: Some(MediaProfile::default()),
            timestamp_ms: issued_at_ms,
            expires_at_ms: issued_at_ms + 5_000,
            nonce: [7; 16],
        },
    )
    .expect("sign untrusted request");
    let bytes = serde_json::to_vec(&LanDiscoveryPacket::SignedRemoteSessionRequest(request))
        .expect("serialize signed request");

    let error = process_lan_discovery_packet(
        &service_socket,
        &app_state,
        &bytes,
        source_socket.local_addr().unwrap(),
    )
    .await
    .expect_err("untrusted controller must be rejected before acknowledgement");

    assert!(error
        .to_string()
        .contains("signed LAN session requester is not trusted"));
    assert!(app_state.sessions.lock().await.get(&session_id).is_none());
    let mut buffer = [0_u8; 1024];
    assert!(timeout(
        Duration::from_millis(50),
        source_socket.recv_from(&mut buffer)
    )
    .await
    .is_err());
}

#[tokio::test]
async fn signed_session_request_rejects_a_relayed_udp_source_address() {
    let app_state = Arc::new(AppState::new());
    app_state.devices.lock().await.register(
        DeviceId("target-device".to_string()),
        "Target Device".to_string(),
    );
    let controller = identity();
    let issued_at_ms = current_time_ms();
    let session_id = SessionId("relayed-inbound-session".to_string());
    let service_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let relay_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let observed_relay = relay_socket.local_addr().unwrap();
    let claimed_controller = SocketAddr::new(
        observed_relay.ip(),
        observed_relay.port().wrapping_add(1).max(1),
    );
    let request = SignedLanSessionRequest::sign(
        &controller,
        LanSessionRequest {
            magic: DISCOVERY_MAGIC.to_string(),
            app_id: DISCOVERY_APP_ID.to_string(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            instance_id: "trusted-controller-instance".to_string(),
            session_id: session_id.0.clone(),
            source_device_id: "trusted-controller-device".to_string(),
            source_device_name: "Trusted Controller".to_string(),
            source_key_id: controller.key_id().to_string(),
            source_key_epoch: 1,
            target_device_id: "target-device".to_string(),
            target_key_id: app_state
                .device_identities
                .machine_key_id()
                .expect("local machine key")
                .to_string(),
            target_key_epoch: app_state
                .device_identities
                .machine_key_epoch()
                .expect("local machine key epoch"),
            transport_kind: "quic".to_string(),
            source_discovery_port: Some(21_116),
            source_endpoint: claimed_controller,
            source_media_capabilities: vec![
                "decode.software".to_string(),
                "quic_stream_media_v2".to_string(),
            ],
            requested_media_profile: Some(MediaProfile::default()),
            timestamp_ms: issued_at_ms,
            expires_at_ms: issued_at_ms + 5_000,
            nonce: [11; 16],
        },
    )
    .expect("sign relayed request");

    let error = process_lan_discovery_packet(
        &service_socket,
        &app_state,
        &serde_json::to_vec(&LanDiscoveryPacket::SignedRemoteSessionRequest(request)).unwrap(),
        observed_relay,
    )
    .await
    .expect_err("relayed signed request must be rejected before trust and session mutation");

    assert!(error.to_string().contains("source endpoint"));
    assert!(app_state.sessions.lock().await.get(&session_id).is_none());
}
