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

/// Windows CPU capture plus OpenH264 adapter used when a shared-GPU transport
/// is not yet available. Encoded units remain in an explicitly bounded queue;
/// raw desktop pixels never cross the adapter boundary.
#[cfg(windows)]
pub struct WindowsDxgiOpenH264CaptureAdapter {
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
        Self {
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
            128 * 1024,
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
        let join = std::thread::Builder::new()
            .name("mrd-agent-dxgi-capture".to_owned())
            .spawn(move || {
                use mrd_capture_dxgi::DxgiDesktopCapture;
                use mrd_encode_openh264::OpenH264Encoder;
                use mrd_pipeline_core::{FrameCapture, VideoEncoder};

                let Ok(mut capture) = DxgiDesktopCapture::new_for_index(display_id) else {
                    return;
                };
                let Ok(mut encoder) = OpenH264Encoder::new_with_bitrate(
                    profile.width as usize,
                    profile.height as usize,
                    profile.fps,
                    profile.bitrate_bps,
                ) else {
                    return;
                };
                let interval =
                    std::time::Duration::from_nanos(1_000_000_000 / u64::from(profile.fps));
                let mut sequence = 0_u64;
                while !thread_stop.load(Ordering::Acquire) {
                    let next_frame = std::time::Instant::now() + interval;
                    let Ok(frame) = capture.capture_frame() else {
                        break;
                    };
                    if thread_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(bounds) = capture.source_bounds() else {
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
                        break;
                    };
                    let Ok(units) = encoder.encode(&frame) else {
                        break;
                    };
                    for unit in units {
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
                            return;
                        };
                        let accepted = queues.lock().ok().and_then(|mut all| {
                            all.get_mut(&resource_id).map(|queue| queue.push(unit))
                        });
                        if accepted != Some(true) {
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
    use super::{CaptureAdapter, WindowsDxgiOpenH264CaptureAdapter};
    use crate::media::{MediaResourceKind, MediaResourceRegistry};
    use mrd_proto::SessionId;

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
        let mut adapter = WindowsDxgiOpenH264CaptureAdapter::new();
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
            if let Some(unit) = adapter.pop_encoded(&id) {
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
