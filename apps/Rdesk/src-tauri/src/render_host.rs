use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use mrd_observability::{ProbeRegistry, StageId};
use mrd_proto::SessionId;
use mrd_render::{
    BoxedRenderer, RenderFrame, RenderPixelFormat, RenderTarget, RendererFactory, RendererSnapshot,
};
use mrd_render_d3d11::D3d11RendererFactory;
use serde::{Deserialize, Serialize};

use crate::frame_sink::{DecodedFrameSink, DEFAULT_SOURCE_ID};

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
    pub surface_count: usize,
    pub attached_surface_ids: Vec<String>,
    pub frame: Option<DecodedFrameSnapshotResponse>,
    pub preview_data_url: Option<String>,
    pub renderer_backend: Option<String>,
    pub renderer_snapshot: Option<RendererSnapshotResponse>,
    pub surface_source_bindings: Vec<SurfaceSourceBindingResponse>,
    pub available_source_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererSnapshotResponse {
    pub attached_to_target: bool,
    pub uploaded_frame_count: u64,
    pub last_width: usize,
    pub last_height: usize,
    pub last_pixel_format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceSourceBindingResponse {
    pub surface_id: String,
    pub source_id: String,
}

pub struct RenderHost {
    renderers: HashMap<SessionId, HashMap<String, BoxedRenderer>>,
    surface_sources: HashMap<SessionId, HashMap<String, String>>,
    frame_sink: Option<Arc<Mutex<DecodedFrameSink>>>,
    probe_registry: Option<ProbeRegistry>,
}

impl RenderHost {
    pub fn with_frame_sink(frame_sink: Arc<Mutex<DecodedFrameSink>>) -> Self {
        Self::with_frame_sink_and_probes(frame_sink, None)
    }

    pub fn with_frame_sink_and_probes(
        frame_sink: Arc<Mutex<DecodedFrameSink>>,
        probe_registry: Option<ProbeRegistry>,
    ) -> Self {
        Self {
            renderers: HashMap::new(),
            surface_sources: HashMap::new(),
            frame_sink: Some(frame_sink),
            probe_registry,
        }
    }

    pub fn attach_session(
        &mut self,
        session_id: SessionId,
        surface_id: String,
        window_handle: isize,
    ) -> Result<(), String> {
        let renderers = self.renderers.entry(session_id.clone()).or_default();
        if !renderers.contains_key(&surface_id) {
            let factory = D3d11RendererFactory;
            let mut renderer = factory
                .create()
                .map_err(|error| format!("create d3d11 renderer failed: {error}"))?;
            renderer
                .attach_target(RenderTarget::WindowHandle(window_handle))
                .map_err(|error| format!("attach renderer target failed: {error}"))?;
            renderers.insert(surface_id.clone(), renderer);
        }
        self.surface_sources
            .entry(session_id)
            .or_default()
            .entry(surface_id)
            .or_insert_with(|| DEFAULT_SOURCE_ID.to_string());
        Ok(())
    }

    pub fn detach_session(&mut self, session_id: &SessionId) {
        self.renderers.remove(session_id);
        self.surface_sources.remove(session_id);
    }

    pub fn detach_surface(&mut self, session_id: &SessionId, surface_id: &str) {
        if let Some(renderers) = self.renderers.get_mut(session_id) {
            renderers.remove(surface_id);
            if renderers.is_empty() {
                self.renderers.remove(session_id);
            }
        }
        if let Some(surface_sources) = self.surface_sources.get_mut(session_id) {
            surface_sources.remove(surface_id);
            if surface_sources.is_empty() {
                self.surface_sources.remove(session_id);
            }
        }
    }

    pub fn bind_surface_source(
        &mut self,
        session_id: &SessionId,
        surface_id: &str,
        source_id: String,
    ) -> Result<(), String> {
        let surface_sources = self
            .surface_sources
            .get_mut(session_id)
            .ok_or_else(|| format!("未找到会话 renderer: {}", session_id.0))?;
        if !surface_sources.contains_key(surface_id) {
            return Err(format!("未找到 surface: {}", surface_id));
        }
        surface_sources.insert(surface_id.to_string(), source_id);
        Ok(())
    }

    pub fn snapshot(&mut self, session_id: &SessionId) -> Result<RenderHostSnapshot, String> {
        let attached_surface_ids = self
            .renderers
            .get(session_id)
            .map(|renderers| renderers.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let attached = !attached_surface_ids.is_empty();
        let surface_count = attached_surface_ids.len();
        let Some(frame_sink) = self.frame_sink.as_ref() else {
            return Ok(RenderHostSnapshot {
                attached,
                surface_count,
                attached_surface_ids,
                frame: None,
                preview_data_url: None,
                renderer_backend: None,
                renderer_snapshot: None,
                surface_source_bindings: Vec::new(),
                available_source_ids: Vec::new(),
            });
        };

        let (frame, latest_frame) = {
            let frame_sink = frame_sink.lock().expect("lock decoded frame sink");
            (
                frame_sink
                    .snapshot(session_id)
                    .map(decoded_frame_snapshot_response),
                frame_sink.latest_frame(session_id).cloned(),
            )
        };
        let available_source_ids = {
            let frame_sink = frame_sink.lock().expect("lock decoded frame sink");
            frame_sink.list_sources(session_id)
        };
        let surface_source_bindings = self
            .surface_sources
            .get(session_id)
            .map(|bindings| {
                bindings
                    .iter()
                    .map(|(surface_id, source_id)| SurfaceSourceBindingResponse {
                        surface_id: surface_id.clone(),
                        source_id: source_id.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if let (Some(surface_renderers), Some(frame_to_upload)) =
            (self.renderers.get_mut(session_id), latest_frame.as_ref())
        {
            let latest_source_frames = {
                let frame_sink = frame_sink.lock().expect("lock decoded frame sink");
                available_source_ids
                    .iter()
                    .filter_map(|source_id| {
                        frame_sink
                            .latest_frame_for_source(session_id, source_id)
                            .cloned()
                            .map(|frame| (source_id.clone(), frame))
                    })
                    .collect::<HashMap<_, _>>()
            };
            for (surface_id, renderer) in surface_renderers.iter_mut() {
                let source_bound_frame = self
                    .surface_sources
                    .get(session_id)
                    .and_then(|bindings| bindings.get(surface_id))
                    .and_then(|source_id| latest_source_frames.get(source_id));
                let render_frame =
                    decoded_frame_to_render_frame(source_bound_frame.unwrap_or(frame_to_upload));
                let bytes = render_frame.data.len();
                let started_at = std::time::Instant::now();
                renderer
                    .upload_frame(render_frame)
                    .map_err(|error| format!("upload latest frame to renderer failed: {error}"))?;
                if let Some(probe_registry) = self.probe_registry.as_ref() {
                    probe_registry
                        .session_handle(session_id.clone(), DEFAULT_SOURCE_ID)
                        .record_stage(StageId::RenderUpload, started_at.elapsed(), bytes, false);
                }
            }
        }

        let preview_data_url =
            decoded_frame_preview_with(frame_sink.as_ref(), session_id.0.clone())?;
        let renderer_snapshot = self
            .renderers
            .get(session_id)
            .and_then(|renderers| renderers.values().next())
            .map(|renderer| renderer_snapshot_response(renderer.snapshot()));

        Ok(RenderHostSnapshot {
            attached,
            surface_count,
            attached_surface_ids,
            frame,
            preview_data_url,
            renderer_backend: self
                .renderers
                .get(session_id)
                .and_then(|renderers| (!renderers.is_empty()).then(|| "d3d11".to_string())),
            renderer_snapshot,
            surface_source_bindings,
            available_source_ids,
        })
    }
}

impl Default for RenderHost {
    fn default() -> Self {
        Self {
            renderers: HashMap::new(),
            surface_sources: HashMap::new(),
            frame_sink: None,
            probe_registry: None,
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
            mrd_decode::PixelFormat::Bgra32 => "Bgra32".to_string(),
            mrd_decode::PixelFormat::D3d11Texture => "D3d11Texture".to_string(),
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
    let Some(rgb) = frame.cpu_bytes() else {
        return Ok(None);
    };

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            rgb,
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
            mrd_decode::PixelFormat::Bgra32 => RenderPixelFormat::Bgra32,
            mrd_decode::PixelFormat::D3d11Texture => RenderPixelFormat::Rgb24,
        },
        data: frame.cpu_bytes().map(|bytes| bytes.to_vec()).unwrap_or_default(),
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
    use super::{RenderHost, SurfaceSourceBindingResponse};
    use crate::frame_sink::{DecodedFrameSink, DEFAULT_SOURCE_ID};
    use mrd_decode::{DecodedFrame, PixelFormat};
    use mrd_proto::SessionId;

    #[test]
    fn attached_session_exposes_preview_snapshot() {
        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        sink.lock().expect("lock frame sink").ingest_frame(
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
            .attach_session(SessionId("session-render".into()), "surface-1".into(), 0)
            .expect("attach session");

        let snapshot = render_host
            .snapshot(&SessionId("session-render".into()))
            .expect("render host snapshot");

        assert!(snapshot.attached);
        assert_eq!(snapshot.surface_count, 1);
        assert_eq!(snapshot.attached_surface_ids, vec!["surface-1".to_string()]);
        assert_eq!(
            snapshot.surface_source_bindings,
            vec![SurfaceSourceBindingResponse {
                surface_id: "surface-1".to_string(),
                source_id: DEFAULT_SOURCE_ID.to_string(),
            }]
        );
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
