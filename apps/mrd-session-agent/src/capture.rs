//! Capture-side adapter boundary for Task 25.

use crate::media::MediaResource;
use mrd_proto::SessionId;

/// Desktop capture implementation owned by the interactive-session agent.
///
/// Implementations must not accept a resource that was not admitted by the
/// grant-bound media registry. They should return `false` on platform failure
/// without exposing native error text to the control plane.
pub trait CaptureAdapter: Send {
    /// Whether this adapter has a production implementation that may accept
    /// capture commands. Per-resource device failures are reported by `start`.
    fn is_available(&self) -> bool;
    /// Start capture for one already-authorized resource.
    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool;
    /// Start using immutable encoder settings from the signed product command.
    fn start_with_profile(
        &mut self,
        resource: &MediaResource,
        session_id: &SessionId,
        _profile: Option<mrd_agent_ipc::AgentCaptureProfile>,
    ) -> bool {
        self.start(resource, session_id)
    }
    /// Pop one encoded frame belonging to this exact live resource.
    fn poll_encoded(
        &mut self,
        _resource_id: &[u8; 16],
    ) -> Option<crate::media::EncodedMediaAccessUnit> {
        None
    }
    /// Whether its worker is still running. Failure ends the authenticated agent.
    fn is_resource_running(&self, _resource_id: &[u8; 16]) -> bool {
        true
    }
    /// Stop capture for the exact resource identity.
    fn stop(&mut self, resource_id: &[u8; 16], session_id: &SessionId) -> bool;
}

/// Explicit placeholder used by production assembly until a capture adapter
/// is enabled. It advertises no capability and rejects every operation.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableCaptureAdapter;

impl CaptureAdapter for UnavailableCaptureAdapter {
    fn is_available(&self) -> bool {
        false
    }

    fn start(&mut self, _resource: &MediaResource, _session_id: &SessionId) -> bool {
        false
    }

    fn stop(&mut self, _resource_id: &[u8; 16], _session_id: &SessionId) -> bool {
        false
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureEncoderBackend {
    Nvenc,
    OpenH264,
}

#[cfg(windows)]
impl CaptureEncoderBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::Nvenc => "nvenc",
            Self::OpenH264 => "openh264",
        }
    }
}

#[cfg(windows)]
#[derive(Clone, Copy)]
enum CaptureEncoderPreference {
    Software,
    PreferNvenc,
}

#[cfg(windows)]
fn initialize_preferred_encoder<E, N, S>(
    profile: mrd_agent_ipc::AgentCaptureProfile,
    nvenc: N,
    software: S,
) -> Result<(CaptureEncoderBackend, E), mrd_pipeline_core::PipelineError>
where
    N: FnOnce(mrd_agent_ipc::AgentCaptureProfile) -> Result<E, mrd_pipeline_core::PipelineError>,
    S: FnOnce(mrd_agent_ipc::AgentCaptureProfile) -> Result<E, mrd_pipeline_core::PipelineError>,
{
    if !profile.is_valid() {
        return Err(mrd_pipeline_core::PipelineError::message(
            "capture_encoder_profile_invalid",
        ));
    }
    match nvenc(profile) {
        Ok(encoder) => Ok((CaptureEncoderBackend::Nvenc, encoder)),
        Err(_) => software(profile).map(|encoder| (CaptureEncoderBackend::OpenH264, encoder)),
    }
}

#[cfg(windows)]
fn initialize_capture_encoder(
    profile: mrd_agent_ipc::AgentCaptureProfile,
    preference: CaptureEncoderPreference,
) -> Result<
    (
        CaptureEncoderBackend,
        Box<dyn mrd_pipeline_core::VideoEncoder>,
    ),
    mrd_pipeline_core::PipelineError,
> {
    let software = |approved: mrd_agent_ipc::AgentCaptureProfile| {
        mrd_encode_openh264::OpenH264Encoder::new_with_bitrate(
            approved.width as usize,
            approved.height as usize,
            approved.fps,
            approved.bitrate_bps,
        )
        .map(|encoder| Box::new(encoder) as Box<dyn mrd_pipeline_core::VideoEncoder>)
    };
    match preference {
        CaptureEncoderPreference::Software => {
            if !profile.is_valid() {
                return Err(mrd_pipeline_core::PipelineError::message(
                    "capture_encoder_profile_invalid",
                ));
            }
            software(profile).map(|encoder| (CaptureEncoderBackend::OpenH264, encoder))
        }
        CaptureEncoderPreference::PreferNvenc => initialize_preferred_encoder(
            profile,
            |approved| {
                // The real Windows constructor configures H.264 Baseline, P1,
                // UltraLowLatency, no B-frame reordering and the exact profile.
                // CPU BGRA submissions complete synchronously; this adapter
                // does not opt into the separate queued shared-texture path.
                mrd_encode_nvenc::NvencH264Encoder::new_max_speed_with_bitrate(
                    approved.width as usize,
                    approved.height as usize,
                    approved.fps,
                    approved.bitrate_bps,
                )
                .map(|encoder| Box::new(encoder) as Box<dyn mrd_pipeline_core::VideoEncoder>)
            },
            software,
        ),
    }
}

/// Production Windows capture: prefer low-latency NVENC in each authorized
/// resource, using OpenH264 when hardware initialization is unavailable.
/// Both choices use the same CPU capture, profile scaling, ownership and
/// bounded publication lifecycle as the software adapter.
#[cfg(windows)]
pub struct WindowsDxgiCaptureAdapter {
    inner: WindowsDxgiOpenH264CaptureAdapter,
}

#[cfg(windows)]
impl WindowsDxgiCaptureAdapter {
    /// Creates an idle NVENC-preferred adapter with no live desktop resources.
    pub fn new() -> Self {
        Self {
            inner: WindowsDxgiOpenH264CaptureAdapter::with_encoder_preference(
                CaptureEncoderPreference::PreferNvenc,
            ),
        }
    }
}

#[cfg(windows)]
impl Default for WindowsDxgiCaptureAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
impl CaptureAdapter for WindowsDxgiCaptureAdapter {
    fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool {
        self.inner.start(resource, session_id)
    }

    fn start_with_profile(
        &mut self,
        resource: &MediaResource,
        session_id: &SessionId,
        profile: Option<mrd_agent_ipc::AgentCaptureProfile>,
    ) -> bool {
        self.inner.start_with_profile(resource, session_id, profile)
    }

    fn poll_encoded(
        &mut self,
        resource_id: &[u8; 16],
    ) -> Option<crate::media::EncodedMediaAccessUnit> {
        self.inner.poll_encoded(resource_id)
    }

    fn is_resource_running(&self, resource_id: &[u8; 16]) -> bool {
        self.inner.is_resource_running(resource_id)
    }

    fn stop(&mut self, resource_id: &[u8; 16], session_id: &SessionId) -> bool {
        self.inner.stop(resource_id, session_id)
    }
}

#[cfg(windows)]
const CAPTURE_MAX_ACCESS_UNIT_BYTES: usize = 128 * 1024;

#[cfg(windows)]
fn report_capture_failure(reason: &'static str) {
    tracing::warn!(reason);
    // The launcher binary has no global tracing subscriber. Keep the same
    // static category observable on its diagnostic stderr without native text.
    eprintln!("mrd-session-agent: {reason}");
}

/// Windows CPU capture plus OpenH264 adapter used when a shared-GPU transport
/// is not yet available. Encoded units remain in an explicitly bounded queue;
/// raw desktop pixels never cross the adapter boundary.
#[cfg(windows)]
pub struct WindowsDxgiOpenH264CaptureAdapter {
    encoder_preference: CaptureEncoderPreference,
    workers: std::collections::HashMap<[u8; 16], CaptureWorker>,
    queues: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<[u8; 16], crate::media::MediaAccessUnitQueue>>,
    >,
}

#[cfg(windows)]
struct CaptureWorker {
    session_id: SessionId,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl WindowsDxgiOpenH264CaptureAdapter {
    /// Creates an idle adapter. Capture workers are created per authorized
    /// resource and are never shared across sessions.
    pub fn new() -> Self {
        Self::with_encoder_preference(CaptureEncoderPreference::Software)
    }

    fn with_encoder_preference(encoder_preference: CaptureEncoderPreference) -> Self {
        Self {
            encoder_preference,
            workers: std::collections::HashMap::new(),
            queues: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Pops one encoded unit for a resource, if a worker has produced one.
    pub fn pop_encoded(
        &self,
        resource_id: &[u8; 16],
    ) -> Option<crate::media::EncodedMediaAccessUnit> {
        self.queues
            .lock()
            .ok()?
            .get_mut(resource_id)
            .and_then(crate::media::MediaAccessUnitQueue::pop)
    }
}

#[cfg(windows)]
impl Default for WindowsDxgiOpenH264CaptureAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(windows)]
impl CaptureAdapter for WindowsDxgiOpenH264CaptureAdapter {
    fn is_available(&self) -> bool {
        true
    }

    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool {
        self.start_with_profile(resource, session_id, None)
    }

    fn start_with_profile(
        &mut self,
        resource: &MediaResource,
        session_id: &SessionId,
        profile: Option<mrd_agent_ipc::AgentCaptureProfile>,
    ) -> bool {
        use std::sync::atomic::{AtomicBool, Ordering};

        let Some(profile) = profile.filter(mrd_agent_ipc::AgentCaptureProfile::is_valid) else {
            return false;
        };
        if resource.session_id() != session_id
            || resource.kind() != crate::media::MediaResourceKind::Capture
            || self.workers.contains_key(resource.resource_id())
            || self.workers.len() >= 2
        {
            return false;
        }
        let Some(queue) = crate::media::MediaAccessUnitQueue::new(
            *resource.resource_id(),
            session_id.clone(),
            3,
            // JSON encodes each byte as up to four characters. Keep payload
            // comfortably below the authenticated IPC frame's one-MiB bound.
            CAPTURE_MAX_ACCESS_UNIT_BYTES,
        ) else {
            return false;
        };
        let resource_id = *resource.resource_id();
        let queues = std::sync::Arc::clone(&self.queues);
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        if queues
            .lock()
            .map_or(true, |mut all| all.insert(resource_id, queue).is_some())
        {
            return false;
        }
        let thread_stop = std::sync::Arc::clone(&stop);
        let worker_session_id = session_id.clone();
        let display_id = resource.display_id();
        let encoder_preference = self.encoder_preference;
        let join = std::thread::Builder::new()
            .name("mrd-agent-dxgi-capture".to_owned())
            .spawn(move || {
                use mrd_capture_dxgi::DxgiDesktopCapture;
                use mrd_pipeline_core::FrameCapture;

                let Ok(mut capture) = DxgiDesktopCapture::new_for_index(display_id) else {
                    report_capture_failure("capture_initialization_failed");
                    return;
                };
                let Ok((backend, mut encoder)) =
                    initialize_capture_encoder(profile, encoder_preference)
                else {
                    report_capture_failure("capture_encoder_initialization_failed");
                    return;
                };
                tracing::info!(
                    encoder_backend = backend.as_str(),
                    width = profile.width,
                    height = profile.height,
                    fps = profile.fps,
                    bitrate_bps = profile.bitrate_bps,
                    "capture_encoder_initialized"
                );
                eprintln!(
                    "mrd-session-agent: capture_encoder_initialized backend={} width={} height={} fps={} bitrate_bps={}",
                    backend.as_str(), profile.width, profile.height, profile.fps, profile.bitrate_bps
                );
                let interval =
                    std::time::Duration::from_nanos(1_000_000_000 / u64::from(profile.fps));
                let mut sequence = 0_u64;
                while !thread_stop.load(Ordering::Acquire) {
                    let next_frame = std::time::Instant::now() + interval;
                    let Ok(frame) = capture.capture_frame() else {
                        report_capture_failure("capture_frame_failed");
                        break;
                    };
                    if thread_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(bounds) = capture.source_bounds() else {
                        report_capture_failure("capture_source_bounds_failed");
                        break;
                    };
                    let (Ok(width), Ok(height)) =
                        (u32::try_from(bounds.width), u32::try_from(bounds.height))
                    else {
                        break;
                    };
                    let bounds = mrd_agent_ipc::CaptureSourceBounds {
                        left: bounds.left,
                        top: bounds.top,
                        width,
                        height,
                    };
                    let Some(frame) = prepare_profile_frame(frame, profile) else {
                        report_capture_failure("capture_profile_frame_invalid");
                        break;
                    };
                    let Ok(units) = encoder.encode(&frame) else {
                        // Runtime encoder failure ends the resource rather
                        // than swapping codec state after signed admission.
                        tracing::warn!(
                            reason = "capture_encode_failed",
                            encoder_backend = backend.as_str()
                        );
                        eprintln!("mrd-session-agent: capture_encode_failed backend={}", backend.as_str());
                        break;
                    };
                    for unit in units {
                        if unit.codec != mrd_pipeline_core::VideoCodec::H264 {
                            report_capture_failure("capture_encoded_codec_invalid");
                            return;
                        }
                        if unit.bytes.len() > CAPTURE_MAX_ACCESS_UNIT_BYTES {
                            tracing::warn!(
                                reason = "capture_encoded_unit_oversized",
                                payload_bytes = unit.bytes.len(),
                                max_payload_bytes = CAPTURE_MAX_ACCESS_UNIT_BYTES
                            );
                            eprintln!(
                                "mrd-session-agent: capture_encoded_unit_oversized payload_bytes={} max_payload_bytes={}",
                                unit.bytes.len(), CAPTURE_MAX_ACCESS_UNIT_BYTES
                            );
                            return;
                        }
                        let Some(next) = sequence.checked_add(1) else {
                            return;
                        };
                        sequence = next;
                        let Some(unit) = crate::media::EncodedMediaAccessUnit::new(
                            resource_id,
                            worker_session_id.clone(),
                            sequence,
                            unit.timestamp_us,
                            unit.is_keyframe,
                            unit.bytes,
                        )
                        .and_then(|unit| unit.with_source_bounds(bounds)) else {
                            report_capture_failure("capture_encoded_unit_invalid");
                            return;
                        };
                        let accepted = queues.lock().ok().and_then(|mut all| {
                            all.get_mut(&resource_id).map(|queue| queue.push(unit))
                        });
                        if accepted != Some(true) {
                            report_capture_failure("capture_encoded_unit_rejected");
                            return;
                        }
                    }
                    std::thread::sleep(
                        next_frame.saturating_duration_since(std::time::Instant::now()),
                    );
                }
            });
        let Ok(join) = join else {
            let _ = self.queues.lock().map(|mut all| all.remove(&resource_id));
            return false;
        };
        self.workers.insert(
            resource_id,
            CaptureWorker {
                session_id: session_id.clone(),
                stop,
                join: Some(join),
            },
        );
        true
    }

    fn poll_encoded(
        &mut self,
        resource_id: &[u8; 16],
    ) -> Option<crate::media::EncodedMediaAccessUnit> {
        self.pop_encoded(resource_id)
    }

    fn is_resource_running(&self, resource_id: &[u8; 16]) -> bool {
        self.workers
            .get(resource_id)
            .is_some_and(|worker| worker.join.as_ref().is_some_and(|join| !join.is_finished()))
    }

    fn stop(&mut self, resource_id: &[u8; 16], session_id: &SessionId) -> bool {
        if self
            .workers
            .get(resource_id)
            .is_none_or(|worker| &worker.session_id != session_id)
        {
            return false;
        }
        let Some(mut worker) = self.workers.remove(resource_id) else {
            return false;
        };
        worker
            .stop
            .store(true, std::sync::atomic::Ordering::Release);
        let joined = worker.join.take().is_none_or(|join| join.join().is_ok());
        if let Ok(mut queues) = self.queues.lock() {
            queues.remove(resource_id);
        }
        joined
    }
}

#[cfg(windows)]
impl Drop for WindowsDxgiOpenH264CaptureAdapter {
    fn drop(&mut self) {
        let resources = self
            .workers
            .iter()
            .map(|(id, worker)| (*id, worker.session_id.clone()))
            .collect::<Vec<_>>();
        for (resource_id, session_id) in resources {
            let _ = self.stop(&resource_id, &session_id);
        }
    }
}

/// Encode exactly the approved dimensions. The product's pointer geometry
/// maps the whole encoded frame to the whole display, so no invisible crop or
/// letterbox offset may be introduced here. Pixels stay in this worker.
#[cfg(windows)]
fn prepare_profile_frame(
    frame: mrd_pipeline_core::CapturedFrame,
    profile: mrd_agent_ipc::AgentCaptureProfile,
) -> Option<mrd_pipeline_core::CapturedFrame> {
    use mrd_pipeline_core::{CapturedFrame, FramePixelFormat};
    if !profile.is_valid() || frame.width == 0 || frame.height == 0 {
        return None;
    }
    let pixel_bytes = match frame.pixel_format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
        FramePixelFormat::Nv12 => return None,
    };
    let source_len = frame
        .width
        .checked_mul(frame.height)?
        .checked_mul(pixel_bytes)?;
    if frame.data.len() != source_len {
        return None;
    }
    let width = profile.width as usize;
    let height = profile.height as usize;
    if (frame.width, frame.height) == (width, height) {
        return Some(frame);
    }
    let mut data = vec![0; width.checked_mul(height)?.checked_mul(pixel_bytes)?];
    for y in 0..height {
        let source_y = y * frame.height / height;
        for x in 0..width {
            let source_x = x * frame.width / width;
            let source = (source_y * frame.width + source_x) * pixel_bytes;
            let target = (y * width + x) * pixel_bytes;
            data[target..target + pixel_bytes]
                .copy_from_slice(&frame.data[source..source + pixel_bytes]);
        }
    }
    Some(CapturedFrame::from_cpu(
        width,
        height,
        frame.pixel_format,
        frame.timestamp_us,
        data,
    ))
}

#[cfg(all(windows, test))]
mod tests {
    use super::{CaptureAdapter, WindowsDxgiCaptureAdapter, WindowsDxgiOpenH264CaptureAdapter};
    use crate::media::{MediaResourceKind, MediaResourceRegistry};
    use mrd_proto::SessionId;

    fn encoder_profile() -> mrd_agent_ipc::AgentCaptureProfile {
        mrd_agent_ipc::AgentCaptureProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_bps: 20_000_000,
        }
    }

    #[test]
    fn available_nvenc_is_selected_without_initializing_software() {
        let profile = encoder_profile();
        let (backend, encoder) = super::initialize_preferred_encoder(
            profile,
            |approved| {
                assert_eq!(approved, profile);
                Ok("hardware")
            },
            |_| panic!("software must not initialize when NVENC is available"),
        )
        .unwrap();
        assert_eq!(backend, super::CaptureEncoderBackend::Nvenc);
        assert_eq!(encoder, "hardware");
    }

    #[test]
    fn unavailable_nvenc_falls_back_without_changing_the_approved_profile() {
        let profile = encoder_profile();
        let attempts = std::cell::RefCell::new(Vec::new());
        let (backend, encoder) = super::initialize_preferred_encoder(
            profile,
            |approved| {
                assert_eq!(approved, profile);
                attempts.borrow_mut().push("nvenc");
                Err(mrd_pipeline_core::PipelineError::message("unavailable"))
            },
            |approved| {
                assert_eq!(approved, profile);
                attempts.borrow_mut().push("openh264");
                Ok("software")
            },
        )
        .unwrap();
        assert_eq!(*attempts.borrow(), ["nvenc", "openh264"]);
        assert_eq!(backend, super::CaptureEncoderBackend::OpenH264);
        assert_eq!(encoder, "software");
    }

    #[test]
    fn invalid_profile_is_rejected_before_either_encoder_initializes() {
        let mut profile = encoder_profile();
        profile.fps = 121;
        let result = super::initialize_preferred_encoder::<(), _, _>(
            profile,
            |_| panic!("invalid grant profile reached NVENC"),
            |_| panic!("invalid grant profile reached OpenH264"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn both_encoder_initialization_failures_reject_the_resource() {
        let result = super::initialize_preferred_encoder::<(), _, _>(
            encoder_profile(),
            |_| {
                Err(mrd_pipeline_core::PipelineError::message(
                    "hardware unavailable",
                ))
            },
            |_| {
                Err(mrd_pipeline_core::PipelineError::message(
                    "software unavailable",
                ))
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn software_fallback_produces_decodable_h264_at_the_approved_dimensions() {
        use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoDecoder, VideoEncoder};
        let profile = mrd_agent_ipc::AgentCaptureProfile {
            width: 128,
            height: 72,
            fps: 60,
            bitrate_bps: 1_000_000,
        };
        let (backend, mut encoder) = super::initialize_preferred_encoder(
            profile,
            |_| {
                Err(mrd_pipeline_core::PipelineError::message(
                    "hardware unavailable",
                ))
            },
            |approved| {
                mrd_encode_openh264::OpenH264Encoder::new_with_bitrate(
                    approved.width as usize,
                    approved.height as usize,
                    approved.fps,
                    approved.bitrate_bps,
                )
            },
        )
        .unwrap();
        assert_eq!(backend, super::CaptureEncoderBackend::OpenH264);
        let frame = CapturedFrame::from_cpu(
            profile.width as usize,
            profile.height as usize,
            FramePixelFormat::Bgra32,
            1000,
            [67, 120, 199, 255].repeat(profile.width as usize * profile.height as usize),
        );
        let units = encoder.encode(&frame).unwrap();
        assert!(!units.is_empty());
        let mut decoder = mrd_decode::H264SoftwareDecoder::new().unwrap();
        for unit in units {
            assert_eq!(unit.codec, mrd_pipeline_core::VideoCodec::H264);
            assert_eq!(unit.timestamp_us, 1000);
            decoder.push_access_unit(&unit.bytes).unwrap();
        }
        let decoded = decoder.drain_decoded_frames();
        assert_eq!(decoded.len(), 1);
        assert_eq!((decoded[0].width, decoded[0].height), (128, 72));
    }

    #[test]
    fn oversized_keyframe_cannot_cross_the_capture_ipc_queue() {
        let session = SessionId("capture-payload-bound".to_owned());
        let id = [0x43; 16];
        let mut queue = crate::media::MediaAccessUnitQueue::new(
            id,
            session.clone(),
            3,
            super::CAPTURE_MAX_ACCESS_UNIT_BYTES,
        )
        .unwrap();
        let unit = crate::media::EncodedMediaAccessUnit::new(
            id,
            session,
            1,
            1000,
            true,
            vec![0; super::CAPTURE_MAX_ACCESS_UNIT_BYTES + 1],
        )
        .unwrap();
        assert!(!queue.push(unit));
        assert!(queue.is_empty());
        assert_eq!(super::CAPTURE_MAX_ACCESS_UNIT_BYTES, 128 * 1024);
    }

    #[test]
    #[ignore = "requires a Windows NVIDIA GPU; synthetic CPU frame only"]
    fn native_nvenc_cpu_h264_is_baseline_and_preserves_the_approved_profile() {
        use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoDecoder};
        let profile = mrd_agent_ipc::AgentCaptureProfile {
            width: 320,
            height: 180,
            fps: 60,
            bitrate_bps: 2_000_000,
        };
        let (backend, mut encoder) = super::initialize_capture_encoder(
            profile,
            super::CaptureEncoderPreference::PreferNvenc,
        )
        .unwrap();
        assert_eq!(backend, super::CaptureEncoderBackend::Nvenc);
        let mut decoder = mrd_decode::H264SoftwareDecoder::new().unwrap();
        for timestamp_us in [1000, 17_666, 34_332] {
            let frame = CapturedFrame::from_cpu(
                profile.width as usize,
                profile.height as usize,
                FramePixelFormat::Bgra32,
                timestamp_us,
                [67, 120, 199, 255].repeat(profile.width as usize * profile.height as usize),
            );
            let units = encoder.encode(&frame).unwrap();
            // The chosen CPU submission is synchronous, with one output for
            // every frame and no queued shared-texture tail to flush on stop.
            assert_eq!(units.len(), 1);
            let unit = &units[0];
            assert_eq!(unit.codec, mrd_pipeline_core::VideoCodec::H264);
            assert_eq!(unit.timestamp_us, timestamp_us);
            assert!(!unit.bytes.is_empty());
            if timestamp_us == 1000 {
                assert!(unit.is_keyframe);
                assert!(
                    unit.bytes
                        .windows(5)
                        .any(|nal| { nal[0..3] == [0, 0, 1] && nal[3] & 31 == 7 && nal[4] == 66 }),
                    "NVENC must emit a Baseline SPS compatible with the approved WAN codec"
                );
            }
            decoder.push_access_unit(&unit.bytes).unwrap();
            let decoded = decoder.drain_decoded_frames();
            assert_eq!(decoded.len(), 1);
            assert_eq!((decoded[0].width, decoded[0].height), (320, 180));
        }
    }

    #[test]
    fn approved_frame_geometry_keeps_full_display_pointer_mapping() {
        use mrd_pipeline_core::{CapturedFrame, FramePixelFormat};
        let profile = mrd_agent_ipc::AgentCaptureProfile {
            width: 4,
            height: 2,
            fps: 30,
            bitrate_bps: 600_000,
        };
        let data = (0..4)
            .flat_map(|y| (0..2).flat_map(move |x| [x, y, 77, 255]))
            .collect();
        let frame = CapturedFrame::from_cpu(2, 4, FramePixelFormat::Bgra32, 1000, data);
        let scaled = super::prepare_profile_frame(frame, profile).unwrap();
        assert_eq!(
            (scaled.width, scaled.height, scaled.timestamp_us),
            (4, 2, 1000)
        );
        assert_eq!(&scaled.data[..4], &[0, 0, 77, 255]);
        assert_eq!(&scaled.data[12..16], &[1, 0, 77, 255]);
        assert_eq!(&scaled.data[28..32], &[1, 2, 77, 255]);
        let truncated = CapturedFrame::from_cpu(2, 4, FramePixelFormat::Bgra32, 1000, vec![0; 2]);
        assert!(super::prepare_profile_frame(truncated, profile).is_none());
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop and display capture"]
    fn native_dxgi_capture_worker_starts_and_stops_without_leaking_a_resource() {
        let session = SessionId("capture-smoke".to_owned());
        let id = [0x41; 16];
        let mut registry = MediaResourceRegistry::new();
        registry.start(id, session.clone(), 0, MediaResourceKind::Capture, None);
        let resource = registry.get(&id).unwrap();
        let mut adapter = WindowsDxgiOpenH264CaptureAdapter::new();
        assert!(adapter.start_with_profile(
            resource,
            &session,
            Some(mrd_agent_ipc::AgentCaptureProfile {
                width: 320,
                height: 180,
                fps: 30,
                bitrate_bps: 600_000
            })
        ));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(adapter.stop(&id, &session));
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop and display capture"]
    fn native_capture_h264_uses_the_approved_profile_dimensions() {
        use mrd_pipeline_core::VideoDecoder;
        let session = SessionId("approved-native-profile".into());
        let id = [0x42; 16];
        let mut registry = MediaResourceRegistry::new();
        registry.start(id, session.clone(), 0, MediaResourceKind::Capture, None);
        let mut adapter = WindowsDxgiCaptureAdapter::new();
        let profile = mrd_agent_ipc::AgentCaptureProfile {
            width: 320,
            height: 180,
            fps: 30,
            bitrate_bps: 600_000,
        };
        assert!(adapter.start_with_profile(registry.get(&id).unwrap(), &session, Some(profile)));
        let mut decoder = mrd_decode::H264SoftwareDecoder::new().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let decoded = loop {
            if let Some(unit) = adapter.poll_encoded(&id) {
                let bounds = unit
                    .source_bounds()
                    .expect("real capture must bind physical source geometry");
                assert!(bounds.is_valid());
                decoder.push_access_unit(unit.payload()).unwrap();
                if let Some(frame) = decoder.drain_decoded_frames().into_iter().next() {
                    break frame;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native capture did not produce a decodable frame"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(adapter.stop(&id, &session));
        assert_eq!(
            (decoded.width, decoded.height),
            (profile.width as usize, profile.height as usize)
        );
    }
}
