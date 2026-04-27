use mrd_pipeline_core::{
    CapturedFrame, EncodedAccessUnit, FrameCapture, FramePixelFormat, PipelineError, VideoCodec,
    VideoEncoder,
};

struct DummyCapture;

impl FrameCapture for DummyCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        Ok(CapturedFrame::from_cpu(
            2,
            2,
            FramePixelFormat::Bgra32,
            42,
            vec![0; 16],
        ))
    }
}

struct DummyEncoder;

impl VideoEncoder for DummyEncoder {
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        Ok(vec![EncodedAccessUnit {
            codec: VideoCodec::H264,
            timestamp_us: frame.timestamp_us,
            is_keyframe: true,
            bytes: frame.data.clone(),
        }])
    }
}

#[test]
fn media_contract_passes_frames_and_access_units_between_traits() {
    let mut capture = DummyCapture;
    let mut encoder = DummyEncoder;

    let frame = capture.capture_frame().expect("captured frame");
    let access_units = encoder.encode(&frame).expect("encoded frame");

    assert_eq!(frame.pixel_format, FramePixelFormat::Bgra32);
    assert_eq!(access_units.len(), 1);
    assert_eq!(access_units[0].codec, VideoCodec::H264);
    assert_eq!(access_units[0].timestamp_us, 42);
}
