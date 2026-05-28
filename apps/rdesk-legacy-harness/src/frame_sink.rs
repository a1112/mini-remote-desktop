use std::collections::HashMap;

use mrd_decode::PixelFormat;
use mrd_pipeline_core::{DecodedFrame, DecodedFrameData};
use mrd_proto::SessionId;

pub const DEFAULT_SOURCE_ID: &str = "session-primary";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrameSnapshot {
    pub frame_count: u64,
    pub width: usize,
    pub height: usize,
    pub pixel_format: PixelFormat,
    pub bytes: usize,
}

fn pixel_format_from_frame(frame: &DecodedFrame) -> PixelFormat {
    match &frame.data {
        DecodedFrameData::CpuRgb24(_) => PixelFormat::Rgb24,
        DecodedFrameData::CpuBgra32(_) | DecodedFrameData::CpuP010 { .. } => PixelFormat::Bgra32,
        DecodedFrameData::CpuNv12 { .. } => PixelFormat::Nv12,
        #[cfg(windows)]
        DecodedFrameData::D3D11SharedNv12 { .. } | DecodedFrameData::D3D11SharedP010 { .. } => {
            PixelFormat::D3d11Texture
        }
    }
}

fn bytes_from_frame(frame: &DecodedFrame) -> usize {
    frame.cpu_bytes().map(|b| b.len()).unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct DecodedFrameSink {
    snapshots: HashMap<SessionId, DecodedFrameSnapshot>,
    latest_frames: HashMap<SessionId, DecodedFrame>,
    source_snapshots: HashMap<(SessionId, String), DecodedFrameSnapshot>,
    latest_source_frames: HashMap<(SessionId, String), DecodedFrame>,
}

impl DecodedFrameSink {
    pub fn ingest_frame(&mut self, session_id: SessionId, frame: DecodedFrame) {
        self.ingest_frame_for_source(session_id, DEFAULT_SOURCE_ID.to_string(), frame);
    }

    pub fn ingest_frame_for_source(
        &mut self,
        session_id: SessionId,
        source_id: String,
        frame: DecodedFrame,
    ) {
        let frame_count = self
            .snapshots
            .get(&session_id)
            .map(|snapshot| snapshot.frame_count + 1)
            .unwrap_or(1);

        let pixel_format = pixel_format_from_frame(&frame);
        let bytes = bytes_from_frame(&frame);
        self.snapshots.insert(
            session_id.clone(),
            DecodedFrameSnapshot {
                frame_count,
                width: frame.width,
                height: frame.height,
                pixel_format,
                bytes,
            },
        );
        self.latest_frames.insert(session_id.clone(), frame);

        let source_key = (session_id.clone(), source_id);
        let source_frame_count = self
            .source_snapshots
            .get(&source_key)
            .map(|snapshot| snapshot.frame_count + 1)
            .unwrap_or(1);

        let latest_frame = &self.latest_frames[&session_id];
        self.source_snapshots.insert(
            source_key.clone(),
            DecodedFrameSnapshot {
                frame_count: source_frame_count,
                width: latest_frame.width,
                height: latest_frame.height,
                pixel_format: pixel_format_from_frame(latest_frame),
                bytes: bytes_from_frame(latest_frame),
            },
        );
        self.latest_source_frames
            .insert(source_key, self.latest_frames[&session_id].clone());
    }

    pub fn snapshot(&self, session_id: &SessionId) -> Option<&DecodedFrameSnapshot> {
        self.snapshots.get(session_id)
    }

    pub fn latest_frame(&self, session_id: &SessionId) -> Option<&DecodedFrame> {
        self.latest_frames.get(session_id)
    }

    pub fn source_snapshot(
        &self,
        session_id: &SessionId,
        source_id: &str,
    ) -> Option<&DecodedFrameSnapshot> {
        self.source_snapshots
            .get(&(session_id.clone(), source_id.to_string()))
    }

    pub fn latest_frame_for_source(
        &self,
        session_id: &SessionId,
        source_id: &str,
    ) -> Option<&DecodedFrame> {
        self.latest_source_frames
            .get(&(session_id.clone(), source_id.to_string()))
    }

    pub fn list_sources(&self, session_id: &SessionId) -> Vec<String> {
        let mut sources = self
            .source_snapshots
            .keys()
            .filter(|(candidate_session_id, _)| candidate_session_id == session_id)
            .map(|(_, source_id)| source_id.clone())
            .collect::<Vec<_>>();
        sources.sort();
        sources
    }
}

#[cfg(test)]
mod tests {
    use mrd_decode::PixelFormat;
    use mrd_pipeline_core::DecodedFrame;
    use mrd_proto::SessionId;

    use super::DecodedFrameSink;

    #[test]
    fn ingesting_frame_updates_session_snapshot() {
        let mut sink = DecodedFrameSink::default();

        sink.ingest_frame(
            SessionId("session-1".into()),
            DecodedFrame::from_cpu_rgb24(640, 360, 0, vec![0; 640 * 360 * 3]),
        );

        let snapshot = sink
            .snapshot(&SessionId("session-1".into()))
            .expect("frame snapshot");

        assert_eq!(snapshot.frame_count, 1);
        assert_eq!(snapshot.width, 640);
        assert_eq!(snapshot.height, 360);
        assert_eq!(snapshot.pixel_format, PixelFormat::Rgb24);
        assert_eq!(snapshot.bytes, 640 * 360 * 3);
    }

    #[test]
    fn later_frames_replace_previous_snapshot_payload() {
        let mut sink = DecodedFrameSink::default();
        let session_id = SessionId("session-2".into());

        sink.ingest_frame(
            session_id.clone(),
            DecodedFrame::from_cpu_rgb24(320, 180, 0, vec![1; 320 * 180 * 3]),
        );
        sink.ingest_frame(
            session_id.clone(),
            DecodedFrame::from_cpu_rgb24(1280, 720, 0, vec![2; 1280 * 720 * 3]),
        );

        let snapshot = sink.snapshot(&session_id).expect("latest snapshot");
        let latest_frame = sink.latest_frame(&session_id).expect("latest frame");

        assert_eq!(snapshot.frame_count, 2);
        assert_eq!(snapshot.width, 1280);
        assert_eq!(snapshot.height, 720);
        assert_eq!(snapshot.bytes, 1280 * 720 * 3);
        assert_eq!(latest_frame.width, 1280);
        assert_eq!(latest_frame.height, 720);
        assert_eq!(latest_frame.cpu_bytes().unwrap().len(), 1280 * 720 * 3);
    }

    #[test]
    fn ingesting_frame_for_source_tracks_source_specific_snapshot() {
        let mut sink = DecodedFrameSink::default();
        let session_id = SessionId("session-3".into());

        sink.ingest_frame_for_source(
            session_id.clone(),
            "video-track-1".into(),
            DecodedFrame::from_cpu_rgb24(800, 600, 0, vec![3; 800 * 600 * 3]),
        );

        let snapshot = sink
            .source_snapshot(&session_id, "video-track-1")
            .expect("source snapshot");

        assert_eq!(snapshot.frame_count, 1);
        assert_eq!(snapshot.width, 800);
        assert_eq!(
            sink.list_sources(&session_id),
            vec!["video-track-1".to_string()]
        );
    }
}
