use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use mrd_proto::SessionId;
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};

use crate::frame_sink::DecodedFrameSink;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrameSnapshotResponse {
    pub frame_count: u64,
    pub width: usize,
    pub height: usize,
    pub pixel_format: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderHostSnapshot {
    pub attached: bool,
    pub frame: Option<DecodedFrameSnapshotResponse>,
    pub preview_data_url: Option<String>,
}

#[derive(Debug, Default)]
pub struct RenderHost {
    attached_sessions: HashSet<SessionId>,
    frame_sink: Option<Arc<Mutex<DecodedFrameSink>>>,
}

impl RenderHost {
    pub fn with_frame_sink(frame_sink: Arc<Mutex<DecodedFrameSink>>) -> Self {
        Self {
            attached_sessions: HashSet::new(),
            frame_sink: Some(frame_sink),
        }
    }

    pub fn attach_session(&mut self, session_id: SessionId) {
        self.attached_sessions.insert(session_id);
    }

    pub fn detach_session(&mut self, session_id: &SessionId) {
        self.attached_sessions.remove(session_id);
    }

    pub fn snapshot(&self, session_id: &SessionId) -> Result<RenderHostSnapshot, String> {
        let attached = self.attached_sessions.contains(session_id);
        let Some(frame_sink) = self.frame_sink.as_ref() else {
            return Ok(RenderHostSnapshot {
                attached,
                frame: None,
                preview_data_url: None,
            });
        };

        let frame = frame_sink
            .lock()
            .expect("lock decoded frame sink")
            .snapshot(session_id)
            .map(decoded_frame_snapshot_response);
        let preview_data_url = decoded_frame_preview_with(frame_sink.as_ref(), session_id.0.clone())?;

        Ok(RenderHostSnapshot {
            attached,
            frame,
            preview_data_url,
        })
    }
}

fn decoded_frame_snapshot_response(
    snapshot: &crate::frame_sink::DecodedFrameSnapshot,
) -> DecodedFrameSnapshotResponse {
    DecodedFrameSnapshotResponse {
        frame_count: snapshot.frame_count,
        width: snapshot.width,
        height: snapshot.height,
        pixel_format: match snapshot.pixel_format {
            mrd_decode::PixelFormat::Rgb24 => "Rgb24".to_string(),
        },
        bytes: snapshot.bytes,
    }
}

fn decoded_frame_preview_with(
    sink: &std::sync::Mutex<DecodedFrameSink>,
    session_id: String,
) -> Result<Option<String>, String> {
    let latest_frame = {
        let sink = sink.lock().expect("lock decoded frame sink");
        sink.latest_frame(&SessionId(session_id)).cloned()
    };

    let Some(frame) = latest_frame else {
        return Ok(None);
    };

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &frame.data,
            frame.width as u32,
            frame.height as u32,
            ColorType::Rgb8.into(),
        )
        .map_err(|error| format!("encode decoded frame preview failed: {error}"))?;

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    Ok(Some(format!("data:image/png;base64,{encoded}")))
}

pub fn render_host_snapshot_with(
    render_host: &std::sync::Mutex<RenderHost>,
    session_id: String,
) -> Result<RenderHostSnapshot, String> {
    render_host
        .lock()
        .expect("lock render host")
        .snapshot(&SessionId(session_id))
}

#[cfg(test)]
mod tests {
    use super::RenderHost;
    use crate::frame_sink::DecodedFrameSink;
    use mrd_decode::{DecodedFrame, PixelFormat};
    use mrd_proto::SessionId;

    #[test]
    fn attached_session_exposes_preview_snapshot() {
        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        sink.lock()
            .expect("lock frame sink")
            .ingest_frame(
                SessionId("session-render".into()),
                DecodedFrame {
                    width: 4,
                    height: 4,
                    pixel_format: PixelFormat::Rgb24,
                    data: vec![128; 4 * 4 * 3],
                },
            );

        let mut render_host = RenderHost::with_frame_sink(sink);
        render_host.attach_session(SessionId("session-render".into()));

        let snapshot = render_host
            .snapshot(&SessionId("session-render".into()))
            .expect("render host snapshot");

        assert!(snapshot.attached);
        assert_eq!(snapshot.frame.as_ref().map(|frame| frame.width), Some(4));
        assert!(snapshot
            .preview_data_url
            .as_deref()
            .is_some_and(|value| value.starts_with("data:image/png;base64,")));
    }
}
