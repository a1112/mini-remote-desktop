use mrd_pipeline_core::VideoCodec;
use mrd_transport_webrtc::H264RtpIngress;

#[test]
fn transport_receiver_reassembles_fua_into_h264_access_unit() {
    let mut ingress = H264RtpIngress::default();

    assert!(ingress
        .push_payload(&[0x7c, 0x85, 0xaa, 0xbb], false, 33_000)
        .is_none());
    let access_unit = ingress
        .push_payload(&[0x7c, 0x45, 0xcc, 0xdd], true, 33_000)
        .expect("completed access unit");

    assert_eq!(access_unit.codec, VideoCodec::H264);
    assert_eq!(access_unit.timestamp_us, 33_000);
    assert!(access_unit.is_keyframe);
    assert_eq!(
        access_unit.bytes,
        vec![0, 0, 0, 1, 0x65, 0xaa, 0xbb, 0xcc, 0xdd]
    );
}

#[test]
fn transport_receiver_accumulates_stap_a_and_fua_until_marker() {
    let mut ingress = H264RtpIngress::default();

    assert!(ingress
        .push_payload(&[24, 0, 2, 0x67, 0x42, 0, 2, 0x68, 0xce], false, 66_000,)
        .is_none());
    assert!(ingress
        .push_payload(&[0x7c, 0x85, 0xaa, 0xbb], false, 66_000)
        .is_none());

    let access_unit = ingress
        .push_payload(&[0x7c, 0x45, 0xcc, 0xdd], true, 66_000)
        .expect("completed access unit");

    assert_eq!(access_unit.codec, VideoCodec::H264);
    assert_eq!(access_unit.timestamp_us, 66_000);
    assert!(access_unit.is_keyframe);
    assert_eq!(
        access_unit.bytes,
        vec![
            0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68, 0xce, 0, 0, 0, 1, 0x65, 0xaa, 0xbb, 0xcc,
            0xdd,
        ]
    );
}
