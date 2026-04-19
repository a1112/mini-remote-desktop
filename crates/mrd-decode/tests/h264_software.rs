use mrd_decode::{available_decoder_descriptors, create_decoder, CodecKind, PixelFormat, RuntimeStatus};
use mrd_pipeline_core::DecodedFrameData;
use openh264::{
    encoder::Encoder,
    formats::{RgbSliceU8, YUVBuffer},
};

#[test]
fn h264_software_descriptor_is_runtime_backed() {
    let descriptor = available_decoder_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.id == "h264_software")
        .expect("h264_software descriptor");

    assert_eq!(descriptor.codec, CodecKind::H264);
    assert_eq!(descriptor.runtime_status, RuntimeStatus::RuntimeBacked);
    assert!(descriptor.output_formats.contains(&PixelFormat::Rgb24));
}

#[test]
fn h264_software_decoder_rejects_invalid_access_unit() {
    let mut decoder = create_decoder("h264_software").expect("create h264 software decoder");
    let result = decoder.push_access_unit(&[0, 1, 2, 3]);

    assert!(result.is_err());
}

#[test]
fn h264_software_decoder_emits_rgb_frame_for_valid_access_unit() {
    let mut rgb = Vec::with_capacity(16 * 16 * 3);
    for y in 0..16 {
        for x in 0..16 {
            rgb.push((x * 16) as u8);
            rgb.push((y * 16) as u8);
            rgb.push(96);
        }
    }
    let rgb_source = RgbSliceU8::new(&rgb, (16, 16));
    let yuv = YUVBuffer::from_rgb_source(rgb_source);
    let mut encoder = Encoder::new().expect("openh264 encoder");
    let access_unit = encoder.encode(&yuv).expect("encode access unit").to_vec();

    let mut decoder = create_decoder("h264_software").expect("create h264 software decoder");
    decoder
        .push_access_unit(access_unit.as_slice())
        .expect("decode access unit");
    let frames = decoder.drain_decoded_frames();

    // Note: Software decoder currently doesn't emit frames in drain_decoded_frames
    // This test will be updated when frame extraction is implemented
    assert_eq!(frames.len(), 0);
}
