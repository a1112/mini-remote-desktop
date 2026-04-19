use mrd_decode::{available_decoder_descriptors, create_decoder};
use mrd_pipeline_core::DecodedFrameData;
use mrd_decode_nvdec::probe_h264_available;
use openh264::{
    encoder::Encoder,
    formats::{RgbSliceU8, YUVBuffer},
};

#[test]
fn nvdec_descriptor_is_listed() {
    let descriptor = available_decoder_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.id == "nvdec")
        .expect("nvdec descriptor");

    assert_eq!(descriptor.id, "nvdec");
}

#[test]
fn create_nvdec_decoder_returns_explicit_result() {
    let result = create_decoder("nvdec");
    if probe_h264_available().is_ok() {
        assert!(
            result.is_ok(),
            "supported nvdec runtime should create a decoder"
        );
    } else {
        assert!(
            result.is_err(),
            "unsupported nvdec runtime should return an error"
        );
    }
}

#[test]
fn nvdec_decoder_roundtrips_valid_access_unit_when_supported() {
    let mut decoder = match create_decoder("nvdec") {
        Ok(decoder) => decoder,
        Err(error) => {
            assert!(
                error.to_string().contains("nvdec")
                    || error.to_string().contains("cuda")
                    || error.to_string().contains("cu")
                    || error.to_string().contains("failed"),
                "unexpected nvdec error: {error}"
            );
            return;
        }
    };

    let access_unit = encoded_access_unit();
    decoder
        .push_access_unit(access_unit.as_slice())
        .expect("valid h264 access unit should decode through nvdec");
    let frames = decoder.drain_decoded_frames();

    assert!(!frames.is_empty(), "nvdec should emit at least one frame");
    assert_eq!(frames[0].width, 128);
    assert_eq!(frames[0].height, 128);
    // Check the data is in CPU RGB24 format
    match &frames[0].data {
        DecodedFrameData::CpuRgb24(data) => {
            assert_eq!(data.len(), 128 * 128 * 3);
        }
        _ => panic!("Expected CpuRgb24 data"),
    }
}

fn encoded_access_unit() -> Vec<u8> {
    let mut rgb = Vec::with_capacity(128 * 128 * 3);
    for y in 0..128 {
        for x in 0..128 {
            rgb.push(x as u8);
            rgb.push(y as u8);
            rgb.push(96);
        }
    }
    let rgb_source = RgbSliceU8::new(&rgb, (128, 128));
    let yuv = YUVBuffer::from_rgb_source(rgb_source);
    let mut encoder = Encoder::new().expect("openh264 encoder");
    encoder.encode(&yuv).expect("encode access unit").to_vec()
}
