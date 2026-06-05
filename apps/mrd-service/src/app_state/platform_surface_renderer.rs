#[cfg(target_os = "macos")]
use mrd_ipc::render_proxy::{
    decode_ack, encode_frame_header, RenderProxyFrameHeader, RenderProxyPixelFormat,
};
use mrd_ipc::AttachedRenderSurface;
#[cfg(any(windows, target_os = "macos"))]
use mrd_render::BoxedRenderer;
#[cfg(windows)]
use mrd_render::RendererFactory;
#[cfg(target_os = "macos")]
use mrd_render::{RenderError, RenderFrame, RenderFrameData, RenderTarget, RendererSnapshot};
#[cfg(windows)]
use mrd_render_d3d11::D3d11RendererFactory;
#[cfg(target_os = "macos")]
use nix::sys::socket::{setsockopt, sockopt};
#[cfg(target_os = "macos")]
use std::io::{self, IoSlice, Read, Write};
#[cfg(target_os = "macos")]
use std::os::unix::net::UnixStream as StdUnixStream;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
const MACOS_RENDER_PROXY_SOCKET_BUFFER_BYTES: usize = 8 * 1024 * 1024;

#[cfg(windows)]
pub(crate) fn surface_backend_matches_platform(backend: &str) -> bool {
    backend == "d3d11"
}

#[cfg(windows)]
pub(crate) fn create_platform_surface_renderer(
    surface: &AttachedRenderSurface,
) -> Result<BoxedRenderer, String> {
    if surface.backend != "d3d11" {
        return Err(format!(
            "unsupported Windows native render backend: {}",
            surface.backend
        ));
    }
    D3d11RendererFactory
        .create()
        .map_err(|error| format!("create D3D11 renderer failed: {error}"))
}

#[cfg(target_os = "macos")]
pub(crate) fn surface_backend_matches_platform(backend: &str) -> bool {
    matches!(backend, "macos" | "metal")
}

#[cfg(target_os = "macos")]
pub(crate) fn create_platform_surface_renderer(
    surface: &AttachedRenderSurface,
) -> Result<BoxedRenderer, String> {
    if !surface_backend_matches_platform(&surface.backend) {
        return Err(format!(
            "unsupported macOS native render backend: {}",
            surface.backend
        ));
    }
    let endpoint = surface.render_proxy_endpoint.clone().ok_or_else(|| {
        format!(
            "macOS render surface {} is missing render proxy endpoint",
            surface.surface_id
        )
    })?;
    Ok(Box::new(MacosRenderProxyRenderer::new(endpoint)))
}

#[cfg(target_os = "macos")]
struct MacosRenderProxyRenderer {
    endpoint: String,
    stream: Option<StdUnixStream>,
    snapshot: RendererSnapshot,
    sequence: u64,
}

#[cfg(target_os = "macos")]
impl MacosRenderProxyRenderer {
    fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            stream: None,
            snapshot: RendererSnapshot {
                attached_to_target: false,
                uploaded_frame_count: 0,
                presented_frame_count: 0,
                present_skipped_count: 0,
                render_queue_replacements: Some(0),
                last_present_status: None,
                low_latency_frame_latency_target: None,
                swap_chain_max_frame_latency: None,
                swap_chain_allow_tearing: None,
                swap_chain_waitable_object: None,
                swap_chain_present_mode: Some("render_proxy".to_string()),
                display_refresh_hz: None,
                render_thread_priority: None,
                waitable_wait_count: None,
                waitable_wait_total_ms: None,
                waitable_timeout_count: None,
                last_waitable_wait_ms: None,
                last_render_prepare_wait_ms: None,
                last_render_shared_resource_ms: None,
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: None,
                last_width: 0,
                last_height: 0,
                last_pixel_format: None,
            },
            sequence: 0,
        }
    }

    fn ensure_stream(&mut self) -> Result<&mut StdUnixStream, RenderError> {
        if self.stream.is_none() {
            let stream = StdUnixStream::connect(&self.endpoint).map_err(|error| {
                RenderError::Message(format!("connect macOS render proxy failed: {error}"))
            })?;
            let timeout = Some(Duration::from_millis(250));
            let _ = stream.set_read_timeout(timeout);
            let _ = stream.set_write_timeout(timeout);
            configure_macos_render_proxy_socket(&stream);
            self.stream = Some(stream);
        }
        self.stream
            .as_mut()
            .ok_or_else(|| RenderError::Message("macOS render proxy stream missing".to_string()))
    }

    fn send_payload(
        &mut self,
        header: &RenderProxyFrameHeader,
        payload: &[u8],
    ) -> Result<mrd_ipc::render_proxy::RenderProxyAck, RenderError> {
        let header_bytes = encode_frame_header(header);
        let mut ack_bytes = [0_u8; mrd_ipc::render_proxy::ACK_LEN];
        let stream = self.ensure_stream()?;
        write_all_vectored_pair(stream, &header_bytes, payload).map_err(|error| {
            RenderError::Message(format!("write macOS render proxy payload failed: {error}"))
        })?;
        stream.read_exact(&mut ack_bytes).map_err(|error| {
            RenderError::Message(format!("read macOS render proxy ack failed: {error}"))
        })?;
        decode_ack(&ack_bytes).map_err(RenderError::Message)
    }

    fn send_payload_with_reconnect(
        &mut self,
        header: &RenderProxyFrameHeader,
        payload: &[u8],
    ) -> Result<mrd_ipc::render_proxy::RenderProxyAck, RenderError> {
        match self.send_payload(header, payload) {
            Ok(ack) => Ok(ack),
            Err(first_error) => {
                self.stream = None;
                self.send_payload(header, payload).map_err(|second_error| {
                    RenderError::Message(format!(
                        "{first_error}; retry after reconnect failed: {second_error}"
                    ))
                })
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn configure_macos_render_proxy_socket(stream: &StdUnixStream) {
    if let Err(error) = setsockopt(
        stream,
        sockopt::SndBuf,
        &MACOS_RENDER_PROXY_SOCKET_BUFFER_BYTES,
    ) {
        tracing::debug!(
            %error,
            "failed to set macOS render proxy socket send buffer"
        );
    }
    if let Err(error) = setsockopt(
        stream,
        sockopt::RcvBuf,
        &MACOS_RENDER_PROXY_SOCKET_BUFFER_BYTES,
    ) {
        tracing::debug!(
            %error,
            "failed to set macOS render proxy socket receive buffer"
        );
    }
}

#[cfg(target_os = "macos")]
fn write_all_vectored_pair<W: Write>(
    writer: &mut W,
    mut first: &[u8],
    mut second: &[u8],
) -> io::Result<()> {
    while !first.is_empty() || !second.is_empty() {
        let slices = [IoSlice::new(first), IoSlice::new(second)];
        let written = writer.write_vectored(&slices)?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write macOS render proxy payload",
            ));
        }
        if written < first.len() {
            first = &first[written..];
        } else {
            let second_written = written.saturating_sub(first.len()).min(second.len());
            first = &[];
            second = &second[second_written..];
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
impl mrd_render::RendererInstance for MacosRenderProxyRenderer {
    fn attach_target(&mut self, target: RenderTarget) -> Result<(), RenderError> {
        let RenderTarget::WindowHandle(window_handle) = target;
        if window_handle == 0 {
            return Err(RenderError::Message(
                "macOS render proxy requires a non-null surface handle".to_string(),
            ));
        }
        self.snapshot.attached_to_target = true;
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError> {
        let width = u32::try_from(frame.width)
            .map_err(|_| RenderError::Message("render proxy frame width overflow".to_string()))?;
        let height = u32::try_from(frame.height)
            .map_err(|_| RenderError::Message("render proxy frame height overflow".to_string()))?;
        let (pixel_format, payload, row_pitch) = match frame.data {
            RenderFrameData::Rgb24(data) => (
                RenderProxyPixelFormat::Rgb24,
                bytes::Bytes::from(data),
                0_usize,
            ),
            RenderFrameData::Bgra32(data) => (
                RenderProxyPixelFormat::Bgra32,
                bytes::Bytes::from(data),
                0_usize,
            ),
            RenderFrameData::Nv12 { data, pitch } => (
                RenderProxyPixelFormat::Nv12,
                bytes::Bytes::from(data),
                pitch,
            ),
            RenderFrameData::Nv12Bytes { data, pitch } => {
                (RenderProxyPixelFormat::Nv12, data, pitch)
            }
            #[cfg(windows)]
            RenderFrameData::D3D11SharedBgra { .. }
            | RenderFrameData::D3D11SharedNv12 { .. }
            | RenderFrameData::D3D11SharedP010 { .. } => {
                return Err(RenderError::Message(
                    "macOS render proxy does not accept D3D11 shared textures".to_string(),
                ))
            }
        };
        let payload_len = u32::try_from(payload.len()).map_err(|_| {
            RenderError::Message("render proxy frame payload length overflow".to_string())
        })?;
        let header = RenderProxyFrameHeader {
            pixel_format,
            width,
            height,
            sequence: self.sequence,
            timestamp_us: 0,
            payload_len,
            row_pitch: u32::try_from(row_pitch)
                .map_err(|_| RenderError::Message("render proxy row pitch overflow".to_string()))?,
        };
        self.sequence = self.sequence.saturating_add(1);
        let ack = self.send_payload_with_reconnect(&header, payload.as_ref())?;
        self.snapshot.uploaded_frame_count = self.snapshot.uploaded_frame_count.saturating_add(1);
        self.snapshot.presented_frame_count = self
            .snapshot
            .presented_frame_count
            .saturating_add(ack.presented_frames);
        self.snapshot.present_skipped_count = self
            .snapshot
            .present_skipped_count
            .saturating_add(ack.present_skips);
        self.snapshot.render_queue_replacements = Some(
            self.snapshot
                .render_queue_replacements
                .unwrap_or_default()
                .saturating_add(ack.queue_replacements),
        );
        self.snapshot.last_present_status = if ack.presented_frames > 0 {
            Some("presented".to_string())
        } else if ack.present_skips > 0 {
            Some("skipped".to_string())
        } else {
            Some("not_presented".to_string())
        };
        record_render_proxy_present_config(&mut self.snapshot, &ack);
        self.snapshot.last_render_prepare_wait_ms =
            finite_positive_duration_ms(ack.decode_duration_ms);
        self.snapshot.last_render_shared_resource_ms =
            finite_positive_duration_ms(ack.draw_present_duration_ms);
        self.snapshot.last_render_wait_for_drawable_ms =
            finite_positive_duration_ms(ack.next_drawable_duration_ms);
        self.snapshot.last_render_encode_commit_ms =
            finite_positive_duration_ms(ack.encode_commit_duration_ms);
        self.snapshot.last_render_draw_present_ms = Some(ack.upload_duration_ms);
        self.snapshot.last_width = frame.width;
        self.snapshot.last_height = frame.height;
        self.snapshot.last_pixel_format = Some(frame.pixel_format);
        Ok(())
    }

    fn upload_h264_access_unit(
        &mut self,
        width: usize,
        height: usize,
        timestamp_us: u64,
        payload: bytes::Bytes,
    ) -> Result<(), RenderError> {
        let width = u32::try_from(width)
            .map_err(|_| RenderError::Message("render proxy H.264 width overflow".to_string()))?;
        let height = u32::try_from(height)
            .map_err(|_| RenderError::Message("render proxy H.264 height overflow".to_string()))?;
        let payload_len = u32::try_from(payload.len()).map_err(|_| {
            RenderError::Message("render proxy H.264 payload length overflow".to_string())
        })?;
        let header = RenderProxyFrameHeader {
            pixel_format: RenderProxyPixelFormat::H264,
            width,
            height,
            sequence: self.sequence,
            timestamp_us,
            payload_len,
            row_pitch: 0,
        };
        self.sequence = self.sequence.saturating_add(1);
        let ack = self.send_payload_with_reconnect(&header, &payload)?;
        self.snapshot.uploaded_frame_count = self.snapshot.uploaded_frame_count.saturating_add(1);
        self.snapshot.presented_frame_count = self
            .snapshot
            .presented_frame_count
            .saturating_add(ack.presented_frames);
        self.snapshot.present_skipped_count = self
            .snapshot
            .present_skipped_count
            .saturating_add(ack.present_skips);
        self.snapshot.render_queue_replacements = Some(
            self.snapshot
                .render_queue_replacements
                .unwrap_or_default()
                .saturating_add(ack.queue_replacements),
        );
        self.snapshot.last_present_status = if ack.presented_frames > 0 {
            Some("presented".to_string())
        } else if ack.present_skips > 0 {
            Some("skipped".to_string())
        } else {
            Some("not_presented".to_string())
        };
        record_render_proxy_present_config(&mut self.snapshot, &ack);
        self.snapshot.last_render_prepare_wait_ms =
            finite_positive_duration_ms(ack.decode_duration_ms);
        self.snapshot.last_render_shared_resource_ms =
            finite_positive_duration_ms(ack.draw_present_duration_ms);
        self.snapshot.last_render_wait_for_drawable_ms =
            finite_positive_duration_ms(ack.next_drawable_duration_ms);
        self.snapshot.last_render_encode_commit_ms =
            finite_positive_duration_ms(ack.encode_commit_duration_ms);
        self.snapshot.last_render_draw_present_ms = Some(ack.upload_duration_ms);
        self.snapshot.last_width = width as usize;
        self.snapshot.last_height = height as usize;
        Ok(())
    }

    fn upload_hevc_access_unit(
        &mut self,
        width: usize,
        height: usize,
        timestamp_us: u64,
        payload: bytes::Bytes,
    ) -> Result<(), RenderError> {
        let width = u32::try_from(width)
            .map_err(|_| RenderError::Message("render proxy HEVC width overflow".to_string()))?;
        let height = u32::try_from(height)
            .map_err(|_| RenderError::Message("render proxy HEVC height overflow".to_string()))?;
        let payload_len = u32::try_from(payload.len()).map_err(|_| {
            RenderError::Message("render proxy HEVC payload length overflow".to_string())
        })?;
        let header = RenderProxyFrameHeader {
            pixel_format: RenderProxyPixelFormat::Hevc,
            width,
            height,
            sequence: self.sequence,
            timestamp_us,
            payload_len,
            row_pitch: 0,
        };
        self.sequence = self.sequence.saturating_add(1);
        let ack = self.send_payload_with_reconnect(&header, &payload)?;
        self.snapshot.uploaded_frame_count = self.snapshot.uploaded_frame_count.saturating_add(1);
        self.snapshot.presented_frame_count = self
            .snapshot
            .presented_frame_count
            .saturating_add(ack.presented_frames);
        self.snapshot.present_skipped_count = self
            .snapshot
            .present_skipped_count
            .saturating_add(ack.present_skips);
        self.snapshot.render_queue_replacements = Some(
            self.snapshot
                .render_queue_replacements
                .unwrap_or_default()
                .saturating_add(ack.queue_replacements),
        );
        self.snapshot.last_present_status = if ack.presented_frames > 0 {
            Some("presented".to_string())
        } else if ack.present_skips > 0 {
            Some("skipped".to_string())
        } else {
            Some("not_presented".to_string())
        };
        record_render_proxy_present_config(&mut self.snapshot, &ack);
        self.snapshot.last_render_prepare_wait_ms =
            finite_positive_duration_ms(ack.decode_duration_ms);
        self.snapshot.last_render_shared_resource_ms =
            finite_positive_duration_ms(ack.draw_present_duration_ms);
        self.snapshot.last_render_wait_for_drawable_ms =
            finite_positive_duration_ms(ack.next_drawable_duration_ms);
        self.snapshot.last_render_encode_commit_ms =
            finite_positive_duration_ms(ack.encode_commit_duration_ms);
        self.snapshot.last_render_draw_present_ms = Some(ack.upload_duration_ms);
        self.snapshot.last_width = width as usize;
        self.snapshot.last_height = height as usize;
        Ok(())
    }

    fn snapshot(&self) -> RendererSnapshot {
        self.snapshot.clone()
    }
}

#[cfg(target_os = "macos")]
fn finite_positive_duration_ms(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[cfg(target_os = "macos")]
fn record_render_proxy_present_config(
    snapshot: &mut RendererSnapshot,
    ack: &mrd_ipc::render_proxy::RenderProxyAck,
) {
    if let Some(max_drawable_count) = ack.max_drawable_count {
        snapshot.swap_chain_max_frame_latency = Some(max_drawable_count);
    }
    if let Some(display_sync_enabled) = ack.display_sync_enabled {
        snapshot.swap_chain_allow_tearing = Some(!display_sync_enabled);
        snapshot.swap_chain_present_mode =
            Some(render_proxy_present_mode(display_sync_enabled).to_string());
    }
}

#[cfg(target_os = "macos")]
fn render_proxy_present_mode(display_sync_enabled: bool) -> &'static str {
    if display_sync_enabled {
        "render_proxy_metal_display_sync"
    } else {
        "render_proxy_metal_immediate"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_surface_backend_matches_only_d3d11() {
        assert!(surface_backend_matches_platform("d3d11"));
        assert!(!surface_backend_matches_platform("metal"));
        assert!(!surface_backend_matches_platform("macos"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_surface_backend_accepts_macos_and_metal() {
        assert!(surface_backend_matches_platform("macos"));
        assert!(surface_backend_matches_platform("metal"));
        assert!(!surface_backend_matches_platform("d3d11"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_render_proxy_vectored_write_preserves_frame_bytes() {
        let (mut writer, mut reader) = StdUnixStream::pair().expect("create stream pair");
        let header = b"MRDR-header";
        let payload = b"nv12-payload";

        write_all_vectored_pair(&mut writer, header, payload).expect("write frame bytes");

        let mut frame = vec![0_u8; header.len() + payload.len()];
        reader.read_exact(&mut frame).expect("read frame bytes");

        let mut expected = Vec::from(header.as_slice());
        expected.extend_from_slice(payload);
        assert_eq!(frame, expected);
    }
}
