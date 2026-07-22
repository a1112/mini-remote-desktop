use mrd_decode::{FfmpegCliDecoder, FfmpegDecodeCodec};
use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{
    CapturedFrame, DecodedFrameData, FramePixelFormat, VideoDecoder, VideoEncoder,
};
use std::{
    thread,
    time::{Duration, Instant},
};

#[test]
fn ffmpeg_decoder_rejects_missing_executable() {
    let missing = std::env::temp_dir().join("mrd-missing-ffmpeg-for-test.exe");

    let error = match FfmpegCliDecoder::new_with_ffmpeg_path(FfmpegDecodeCodec::H264, missing) {
        Ok(_) => panic!("missing FFmpeg executable should be rejected"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("FFmpeg executable"));
}

#[test]
fn ffmpeg_h264_decoder_decodes_openh264_access_unit_when_tool_available() {
    let probe = mrd_ffmpeg::probe_ffmpeg(&mrd_ffmpeg::golden_settings());
    let Some(ffmpeg_path) = probe.ffmpeg_path else {
        return;
    };

    let width = 32;
    let height = 32;
    let mut encoder = OpenH264Encoder::new(width, height, 30).expect("create OpenH264 encoder");
    let mut decoder = FfmpegCliDecoder::new_with_ffmpeg_path(FfmpegDecodeCodec::H264, ffmpeg_path)
        .expect("create FFmpeg H.264 decoder");
    for index in 0..8 {
        let frame = CapturedFrame::from_cpu(
            width,
            height,
            FramePixelFormat::Bgra32,
            index * 33_333,
            vec![127 + index as u8; width * height * 4],
        );
        let access_units = encoder.encode(&frame).expect("encode H.264 frame");
        for access_unit in access_units {
            decoder
                .push_access_unit(&access_unit.bytes)
                .expect("decode H.264 access unit");
        }
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut frames = Vec::new();
    while frames.is_empty() && Instant::now() < deadline {
        frames = decoder.drain_decoded_frames();
        if frames.is_empty() {
            thread::sleep(Duration::from_millis(50));
        }
    }

    assert!(!frames.is_empty(), "FFmpeg decoder produced no frames");
    assert_eq!(frames[0].width, width);
    assert_eq!(frames[0].height, height);
    match &frames[0].data {
        DecodedFrameData::CpuNv12 { data, pitch } => {
            assert_eq!(*pitch, width);
            assert_eq!(data.len(), width * height * 3 / 2);
        }
        other => panic!("expected CpuNv12 frame, got {other:?}"),
    }
}
