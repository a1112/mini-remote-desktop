use bytes::Bytes;
use mrd_transport_quic_quinn::{
    fragment_access_unit, fragment_media_payload_v3, QuicAuFragment, QuicAuReassembler,
    QuicAuReassemblerConfig, QuicMediaCodec, QuicMediaFragment, QuicMediaPayloadType,
    QuicMediaReassembler, QUIC_AU_FRAGMENT_HEADER_LEN, QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN,
};

fn media_fragments(payload: &[u8], max_size: usize) -> Vec<Bytes> {
    fragment_media_payload_v3(
        QuicMediaPayloadType::AccessUnit,
        QuicMediaCodec::Hevc,
        3,
        7,
        123_456,
        true,
        payload,
        max_size,
    )
    .unwrap()
}

#[test]
fn owned_fragment_decode_retains_the_datagram_payload() {
    let legacy = fragment_access_unit(7, 123_456, true, b"payload", 1200)
        .unwrap()
        .remove(0);
    let media = media_fragments(b"payload", 1200).remove(0);
    let legacy_payload = legacy.slice(QUIC_AU_FRAGMENT_HEADER_LEN..);
    let media_payload = media.slice(QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN..);

    let legacy_decoded = QuicAuFragment::decode_owned(legacy.clone()).unwrap();
    let media_decoded = QuicMediaFragment::decode_owned(media.clone()).unwrap();
    assert_eq!(legacy_decoded, QuicAuFragment::decode(&legacy).unwrap());
    assert_eq!(media_decoded, QuicMediaFragment::decode(&media).unwrap());
    assert_eq!(legacy_decoded.payload.as_ptr(), legacy_payload.as_ptr());
    assert_eq!(media_decoded.payload.as_ptr(), media_payload.as_ptr());
}

#[test]
fn owned_single_fragment_reassembly_moves_the_payload() {
    let legacy = fragment_access_unit(7, 123_456, true, b"payload", 1200)
        .unwrap()
        .remove(0);
    let media = media_fragments(b"payload", 1200).remove(0);
    let legacy_payload = legacy.slice(QUIC_AU_FRAGMENT_HEADER_LEN..);
    let media_payload = media.slice(QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN..);
    let mut legacy_receiver = QuicAuReassembler::default();
    let mut media_receiver = QuicMediaReassembler::default();
    let legacy_frame = legacy_receiver
        .push_datagram_owned(legacy)
        .unwrap()
        .unwrap();
    let media_frame = media_receiver.push_datagram_owned(media).unwrap().unwrap();
    assert_eq!(legacy_frame.payload.as_ptr(), legacy_payload.as_ptr());
    assert_eq!(media_frame.payload.as_ptr(), media_payload.as_ptr());
    assert_eq!(legacy_receiver.stats().completed_frames, 1);
    assert_eq!(media_receiver.stats().completed_frames, 1);
    assert_eq!(legacy_receiver.stats().pending_bytes, 0);
    assert_eq!(media_receiver.stats().pending_bytes, 0);
}

#[test]
fn fragmentation_preserves_the_v2_and_v3_wire_layout() {
    let legacy = fragment_access_unit(7, 123_456, true, &[0x55, 0xaa, 0xbb], 19).unwrap();
    assert_eq!(legacy.len(), 2);
    assert_eq!(
        legacy[0].as_ref(),
        &[7, 0, 0, 0, 64, 226, 1, 0, 0, 0, 0, 0, 1, 0, 0, 2, 0, 0x55, 0xaa,]
    );
    assert_eq!(
        legacy[1].as_ref(),
        &[7, 0, 0, 0, 64, 226, 1, 0, 0, 0, 0, 0, 1, 1, 0, 2, 0, 0xbb,]
    );
    let media = media_fragments(&[0x55, 0xaa, 0xbb], 34);
    assert_eq!(media.len(), 2);
    assert_eq!(
        media[0].as_ref(),
        &[
            0x4d, 0x52, 0x44, 0x33, 3, 1, 2, 1, 3, 0, 0, 0, 7, 0, 0, 0, 64, 226, 1, 0, 0, 0, 0, 0,
            0, 0, 2, 0, 2, 0, 0, 0, 0x55, 0xaa,
        ]
    );
    assert_eq!(
        QuicMediaFragment::decode(&media[1])
            .unwrap()
            .payload
            .as_ref(),
        &[0xbb]
    );
}

#[test]
fn owned_reassembly_preserves_out_of_order_frames_and_duplicates() {
    let payload = vec![0x65; 3000];
    let legacy = fragment_access_unit(7, 123_456, true, &payload, 512).unwrap();
    let media = media_fragments(&payload, 512);
    let mut legacy_receiver = QuicAuReassembler::default();
    let mut media_receiver = QuicMediaReassembler::default();
    assert!(legacy_receiver
        .push_datagram_owned(legacy[0].clone())
        .unwrap()
        .is_none());
    assert!(legacy_receiver
        .push_datagram_owned(legacy[0].clone())
        .unwrap()
        .is_none());
    assert!(media_receiver
        .push_datagram_owned(media[0].clone())
        .unwrap()
        .is_none());
    assert!(media_receiver
        .push_datagram_owned(media[0].clone())
        .unwrap()
        .is_none());
    let mut legacy_completed = None;
    let mut media_completed = None;
    for fragment in legacy.into_iter().skip(1).rev() {
        legacy_completed = legacy_receiver
            .push_datagram_owned(fragment)
            .unwrap()
            .or(legacy_completed);
    }
    for fragment in media.into_iter().skip(1).rev() {
        media_completed = media_receiver
            .push_datagram_owned(fragment)
            .unwrap()
            .or(media_completed);
    }
    assert_eq!(legacy_completed.unwrap().payload.as_ref(), payload);
    assert_eq!(media_completed.unwrap().payload.as_ref(), payload);
    assert_eq!(legacy_receiver.stats().duplicate_fragments, 1);
    assert_eq!(media_receiver.stats().duplicate_fragments, 1);
}

#[test]
fn owned_single_fragment_cannot_bypass_byte_limits() {
    let legacy = fragment_access_unit(7, 123_456, true, b"payload", 1200)
        .unwrap()
        .remove(0);
    let media = media_fragments(b"payload", 1200).remove(0);
    let mut legacy_receiver =
        QuicAuReassembler::new(QuicAuReassemblerConfig::default()).with_max_frame_bytes(3);
    let mut media_receiver =
        QuicMediaReassembler::new(QuicAuReassemblerConfig::default()).with_max_frame_bytes(3);
    assert!(legacy_receiver.push_datagram_owned(legacy).is_err());
    assert!(media_receiver.push_datagram_owned(media).is_err());
    assert_eq!(legacy_receiver.stats().rejected_fragments, 1);
    assert_eq!(media_receiver.stats().rejected_fragments, 1);
    assert_eq!(legacy_receiver.stats().completed_frames, 0);
    assert_eq!(media_receiver.stats().completed_frames, 0);
}

#[test]
fn owned_decode_keeps_wire_validation_and_empty_payload_support() {
    let legacy = fragment_access_unit(7, 123_456, true, b"", 1200)
        .unwrap()
        .remove(0);
    let media = media_fragments(b"", 1200).remove(0);
    assert!(QuicAuReassembler::default()
        .push_datagram_owned(legacy.clone())
        .unwrap()
        .unwrap()
        .payload
        .is_empty());
    assert!(QuicMediaReassembler::default()
        .push_datagram_owned(media.clone())
        .unwrap()
        .unwrap()
        .payload
        .is_empty());
    let mut bad_legacy = legacy.to_vec();
    bad_legacy[15..17].copy_from_slice(&0_u16.to_le_bytes());
    assert!(QuicAuFragment::decode_owned(bad_legacy.into()).is_err());
    let mut bad_media = media.to_vec();
    bad_media[28..32].copy_from_slice(&1_u32.to_le_bytes());
    assert!(QuicMediaFragment::decode_owned(bad_media.into()).is_err());
}
