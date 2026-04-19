//! End-to-end media pipeline integration tests
//!
//! Tests the complete flow: Capture → Encode → Transport → Decode → Render
//!
//! NOTE: These tests require Windows and NVIDIA GPU for full functionality.
//! Most tests use mock components to verify pipeline structure.

use std::time::Duration;

use mrd_pipeline_core::{CapturedFrame, EncodedAccessUnit, FrameCapture, PipelineError, VideoCodec, VideoEncoder};

// Mock components for pipeline testing
mod mock {
    use super::*;

    #[derive(Debug)]
    pub struct MockCapture {
        width: usize,
        height: usize,
        frame_count: usize,
        max_frames: Option<usize>,
    }

    impl MockCapture {
        pub fn new(width: usize, height: usize) -> Self {
            Self {
                width,
                height,
                frame_count: 0,
                max_frames: None,
            }
        }

        pub fn with_max_frames(mut self, max: usize) -> Self {
            self.max_frames = Some(max);
            self
        }
    }

    impl FrameCapture for MockCapture {
        fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
            if let Some(max) = self.max_frames {
                if self.frame_count >= max {
                    return Err(PipelineError::message("No more frames available"));
                }
            }

            let timestamp_us = self.frame_count as u64 * 33_333; // ~30fps
            self.frame_count += 1;

            Ok(CapturedFrame {
                width: self.width,
                height: self.height,
                pixel_format: mrd_pipeline_core::FramePixelFormat::Bgra32,
                timestamp_us,
                data: vec![0x55; self.width * self.height * 4],
            })
        }
    }

    #[derive(Debug)]
    pub struct MockEncoder {
        frame_delay: Option<Duration>,
        keyframe_interval: usize,
        frame_count: usize,
    }

    impl MockEncoder {
        pub fn new() -> Self {
            Self {
                frame_delay: None,
                keyframe_interval: 30,
                frame_count: 0,
            }
        }

        pub fn with_delay(mut self, delay: Duration) -> Self {
            self.frame_delay = Some(delay);
            self
        }

        pub fn with_keyframe_interval(mut self, interval: usize) -> Self {
            self.keyframe_interval = interval;
            self
        }
    }

    impl Default for MockEncoder {
        fn default() -> Self {
            Self::new()
        }
    }

    impl VideoEncoder for MockEncoder {
        fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            if let Some(delay) = self.frame_delay {
                std::thread::sleep(delay);
            }

            let is_keyframe = self.frame_count % self.keyframe_interval == 0;
            self.frame_count += 1;

            // Simulate H.264 Annex-B format
            let mut data = vec![0x00, 0x00, 0x00, 0x01]; // Annex-B start code
            data.push(if is_keyframe { 0x67 } else { 0x41 }); // NAL type
            data.extend_from_slice(&frame.data[..frame.data.len().min(100)]); // Truncated data

            Ok(vec![EncodedAccessUnit {
                codec: VideoCodec::H264,
                timestamp_us: frame.timestamp_us,
                is_keyframe,
                bytes: data,
            }])
        }
    }

    #[derive(Debug)]
    pub struct MockDecoder {
        decode_delay: Option<Duration>,
    }

    impl MockDecoder {
        pub fn new() -> Self {
            Self {
                decode_delay: None,
            }
        }

        pub fn with_delay(mut self, delay: Duration) -> Self {
            self.decode_delay = Some(delay);
            self
        }
    }

    impl Default for MockDecoder {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockDecoder {
        pub fn decode(&mut self, _access_unit: &EncodedAccessUnit) -> Result<Vec<u8>, PipelineError> {
            if let Some(delay) = self.decode_delay {
                std::thread::sleep(delay);
            }

            // Simulate decoded frame (RGB)
            let width = 128;
            let height = 128;
            Ok(vec![0x77; width * height * 3])
        }
    }

    #[derive(Debug)]
    pub struct MockRenderer {
        frame_count: usize,
        max_frames: Option<usize>,
    }

    impl MockRenderer {
        pub fn new() -> Self {
            Self {
                frame_count: 0,
                max_frames: None,
            }
        }

        pub fn with_max_frames(mut self, max: usize) -> Self {
            self.max_frames = Some(max);
            self
        }

        pub fn render(&mut self, _decoded_frame: &[u8]) -> Result<(), PipelineError> {
            if let Some(max) = self.max_frames {
                if self.frame_count >= max {
                    return Err(PipelineError::message("Renderer buffer full"));
                }
            }

            self.frame_count += 1;
            Ok(())
        }

        pub fn frame_count(&self) -> usize {
            self.frame_count
        }
    }

    impl Default for MockRenderer {
        fn default() -> Self {
            Self::new()
        }
    }
}

// Re-export mock types
pub use mock::{MockCapture, MockDecoder, MockEncoder, MockRenderer};

/// End-to-end pipeline test using mock components
#[test]
fn e2e_mock_pipeline_processes_multiple_frames() {
    let mut capture = MockCapture::new(128, 128).with_max_frames(10);
    let mut encoder = MockEncoder::new();
    let mut decoder = MockDecoder::new();
    let mut renderer = MockRenderer::new().with_max_frames(10);

    let frame_count = 10;

    for _ in 0..frame_count {
        // Capture
        let captured = capture
            .capture_frame()
            .expect("Failed to capture frame");

        // Encode
        let encoded = encoder
            .encode(&captured)
            .expect("Failed to encode frame");
        assert_eq!(encoded.len(), 1);
        let access_unit = &encoded[0];

        // Verify codec
        assert_eq!(access_unit.codec, VideoCodec::H264);

        // Decode
        let decoded = decoder
            .decode(access_unit)
            .expect("Failed to decode frame");

        // Render
        renderer
            .render(&decoded)
            .expect("Failed to render frame");
    }

    assert_eq!(renderer.frame_count(), frame_count);
}

/// Test keyframe detection in encoded access units
#[test]
fn e2e_pipeline_detects_keyframes() {
    let mut capture = MockCapture::new(128, 128);
    let mut encoder = MockEncoder::new().with_keyframe_interval(5);

    let mut keyframe_count = 0;

    for _ in 0..10 {
        let captured = capture.capture_frame().expect("Capture failed");
        let encoded = encoder.encode(&captured).expect("Encode failed");

        if encoded[0].is_keyframe {
            keyframe_count += 1;
        }
    }

    assert_eq!(keyframe_count, 2); // Frames 0 and 5
}

/// Test pipeline error propagation
#[test]
fn e2e_pipeline_propagates_capture_errors() {
    let mut capture = MockCapture::new(128, 128).with_max_frames(0);
    let mut encoder = MockEncoder::new();

    let result = capture.capture_frame()
        .and_then(|frame| encoder.encode(&frame).map(|_| ()));

    assert!(result.is_err());
    assert_eq!(result.unwrap_err().to_string(), "No more frames available");
}

/// Test pipeline with delay simulation (latency measurement)
#[test]
fn e2e_pipeline_measures_total_latency() {
    let encode_delay = Duration::from_millis(2);
    let decode_delay = Duration::from_millis(1);

    let mut capture = MockCapture::new(128, 128).with_max_frames(5);
    let mut encoder = MockEncoder::new().with_delay(encode_delay);
    let mut decoder = MockDecoder::new().with_delay(decode_delay);

    let start = std::time::Instant::now();

    for _ in 0..5 {
        let captured = capture.capture_frame().expect("Capture failed");
        let encoded = encoder.encode(&captured).expect("Encode failed");
        decoder.decode(&encoded[0]).expect("Decode failed");
    }

    let elapsed = start.elapsed();

    // Expected: (2 + 1) * 5 = 15ms minimum
    // Allow some margin for test execution overhead
    assert!(elapsed >= Duration::from_millis(10));
    assert!(elapsed < Duration::from_millis(100)); // Should complete quickly
}

/// Test session lifecycle with mock pipeline
#[test]
fn e2e_session_lifecycle_creates_streams_and_destroys() {
    use mrd_proto::{DeviceId, SessionId};
    use mrd_session::{QuicSessionCoordinator, SessionLifecycleState};

    let mut coordinator = QuicSessionCoordinator::default();
    let session_id = SessionId("test-session-1".to_string());

    // Create session
    coordinator.request_session(
        session_id.clone(),
        DeviceId("controller-1".to_string()),
        DeviceId("agent-1".to_string()),
        "quic_quinn".to_string(),
        Some("127.0.0.1:5000".to_string()),
        Some("localhost".to_string()),
        Some("AQID".to_string()),
    ).expect("Failed to request session");

    let snapshot = coordinator.snapshot(&session_id).expect("Session not found");
    assert_eq!(snapshot.lifecycle_state, SessionLifecycleState::Connecting);

    // Connect
    coordinator.set_connected(&session_id).expect("Failed to connect");
    let snapshot = coordinator.snapshot(&session_id).expect("Session not found");
    assert_eq!(snapshot.lifecycle_state, SessionLifecycleState::Connected);

    // Stream
    coordinator.set_streaming(&session_id).expect("Failed to start streaming");
    let snapshot = coordinator.snapshot(&session_id).expect("Session not found");
    assert_eq!(snapshot.lifecycle_state, SessionLifecycleState::Streaming);

    // Close
    coordinator.close(&session_id).expect("Failed to close session");
    let snapshot = coordinator.snapshot(&session_id).expect("Session not found");
    assert_eq!(snapshot.lifecycle_state, SessionLifecycleState::Closed);
}

/// Test that pipeline handles timestamp progression correctly
#[test]
fn e2e_pipeline_maintains_timestamp_order() {
    let mut capture = MockCapture::new(128, 128).with_max_frames(10);
    let mut encoder = MockEncoder::new();

    let mut timestamps = Vec::new();

    for _ in 0..10 {
        let captured = capture.capture_frame().expect("Capture failed");
        timestamps.push(captured.timestamp_us);

        let encoded = encoder.encode(&captured).expect("Encode failed");
        assert_eq!(encoded[0].timestamp_us, captured.timestamp_us);
    }

    // Verify timestamps are monotonically increasing
    for i in 1..timestamps.len() {
        assert!(timestamps[i] > timestamps[i - 1]);
    }
}

/// Test pipeline with different frame sizes
#[test]
fn e2e_pipeline_handles_various_frame_sizes() {
    let sizes = [(64, 64), (128, 128), (320, 240), (640, 480)];

    for (width, height) in sizes {
        let mut capture = MockCapture::new(width, height).with_max_frames(1);
        let mut encoder = MockEncoder::new();
        let mut decoder = MockDecoder::new();
        let mut renderer = MockRenderer::new();

        let captured = capture.capture_frame().expect("Capture failed");
        assert_eq!(captured.width, width);
        assert_eq!(captured.height, height);

        let encoded = encoder.encode(&captured).expect("Encode failed");
        let decoded = decoder.decode(&encoded[0]).expect("Decode failed");
        renderer.render(&decoded).expect("Render failed");
    }
}

/// Test frame data integrity through the pipeline
#[test]
fn e2e_pipeline_preserves_frame_metadata() {
    let mut capture = MockCapture::new(640, 480).with_max_frames(1);
    let mut encoder = MockEncoder::new();

    let captured = capture.capture_frame().expect("Capture failed");
    let encoded = encoder.encode(&captured).expect("Encode failed");

    assert_eq!(encoded[0].timestamp_us, captured.timestamp_us);
    assert_eq!(encoded[0].codec, VideoCodec::H264);
}
