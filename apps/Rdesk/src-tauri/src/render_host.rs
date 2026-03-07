use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use mrd_proto::SessionId;
use mrd_render::{
    BoxedRenderer, RenderFrame, RenderPixelFormat, RenderTarget, RendererFactory, RendererSnapshot,
};
use mrd_render_d3d11::D3d11RendererFactory;
use serde::{Deserialize, Serialize};

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
    pub renderer_backend: Option<String>,
    pub renderer_snapshot: Option<RendererSnapshotResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererSnapshotResponse {
    pub attached_to_target: bool,
    pub uploaded_frame_count: u64,
    pub last_width: usize,
    pub last_height: usize,
    pub last_pixel_format: Option<String>,
}

pub struct RenderHost {
    attached_sessions: HashSet<SessionId>,
    renderers: HashMap<SessionId, BoxedRenderer>,
    frame_sink: Option<Arc<Mutex<DecodedFrameSink>>>,
}

impl RenderHost {
    pub fn with_frame_sink(frame_sink: Arc<Mutex<DecodedFrameSink>>) -> Self {
        Self {
            attached_sessions: HashSet::new(),
            renderers: HashMap::new(),
            frame_sink: Some(frame_sink),
        }
    }

    pub fn attach_session(
        &mut self,
        session_id: SessionId,
        window_handle: isize,
    ) -> Result<(), String> {
        self.attached_sessions.insert(session_id.clone());
        if !self.renderers.contains_key(&session_id) {
            let factory = D3d11RendererFactory;
            let mut renderer = factory
                .create()
                .map_err(|error| format!("create d3d11 renderer failed: {error}"))?;
            renderer
                .attach_target(RenderTarget::WindowHandle(window_handle))
                .map_err(|error| format!("attach renderer target failed: {error}"))?;
            self.renderers.insert(session_id, renderer);
        }
        Ok(())
    }

    pub fn detach_session(&mut self, session_id: &SessionId) {
        self.attached_sessions.remove(session_id);
        self.renderers.remove(session_id);
    }

    pub fn snapshot(&mut self, session_id: &SessionId) -> Result<RenderHostSnapshot, String> {
        let attached = self.attached_sessions.contains(session_id);
        let Some(frame_sink) = self.frame_sink.as_ref() else {
            return Ok(RenderHostSnapshot {
                attached,
                frame: None,
                preview_data_url: None,
                renderer_backend: None,
                renderer_snapshot: None,
            });
        };

        let (frame, latest_frame) = {
            let frame_sink = frame_sink.lock().expect("lock decoded frame sink");
            (
                frame_sink.snapshot(session_id).map(decoded_frame_snapshot_response),
                frame_sink.latest_frame(session_id).cloned(),
            )
        };

        if let (Some(renderer), Some(frame_to_upload)) =
            (self.renderers.get_mut(session_id), latest_frame.as_ref())
        {
            renderer
                .upload_frame(decoded_frame_to_render_frame(frame_to_upload))
                .map_err(|error| format!("upload latest frame to renderer failed: {error}"))?;
        }

        let preview_data_url = decoded_frame_preview_with(frame_sink.as_ref(), session_id.0.clone())?;
        let renderer_snapshot = self
            .renderers
            .get(session_id)
            .map(|renderer| renderer_snapshot_response(renderer.snapshot()));

        Ok(RenderHostSnapshot {
            attached,
            frame,
            preview_data_url,
            renderer_backend: self
                .renderers
                .get(session_id)
                .map(|_| "d3d11".to_string()),
            renderer_snapshot,
        })
    }
}

impl Default for RenderHost {
    fn default() -> Self {
        Self {
            attached_sessions: HashSet::new(),
            renderers: HashMap::new(),
            frame_sink: None,
        }
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

fn decoded_frame_to_render_frame(frame: &mrd_decode::DecodedFrame) -> RenderFrame {
    RenderFrame {
        width: frame.width,
        height: frame.height,
        pixel_format: match frame.pixel_format {
            mrd_decode::PixelFormat::Rgb24 => RenderPixelFormat::Rgb24,
        },
        data: frame.data.clone(),
    }
}

fn renderer_snapshot_response(snapshot: RendererSnapshot) -> RendererSnapshotResponse {
    RendererSnapshotResponse {
        attached_to_target: snapshot.attached_to_target,
        uploaded_frame_count: snapshot.uploaded_frame_count,
        last_width: snapshot.last_width,
        last_height: snapshot.last_height,
        last_pixel_format: snapshot.last_pixel_format.map(|format| match format {
            RenderPixelFormat::Rgb24 => "Rgb24".to_string(),
        }),
    }
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
        render_host
            .attach_session(SessionId("session-render".into()), 0)
            .expect("attach session");

        let snapshot = render_host
            .snapshot(&SessionId("session-render".into()))
            .expect("render host snapshot");

        assert!(snapshot.attached);
        assert_eq!(snapshot.frame.as_ref().map(|frame| frame.width), Some(4));
        assert_eq!(snapshot.renderer_backend.as_deref(), Some("d3d11"));
        assert_eq!(
            snapshot
                .renderer_snapshot
                .as_ref()
                .map(|renderer| renderer.uploaded_frame_count),
            Some(1)
        );
        assert!(snapshot
            .preview_data_url
            .as_deref()
            .is_some_and(|value| value.starts_with("data:image/png;base64,")));
    }
}
