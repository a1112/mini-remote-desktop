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
    /// Stop capture for the exact resource identity.
    fn stop(&mut self, resource_id: &[u8; 16], session_id: &SessionId) -> bool;
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
        use std::sync::atomic::{AtomicBool, Ordering};

        if self.workers.contains_key(resource.resource_id()) {
            return false;
        }
        let Some(queue) = crate::media::MediaAccessUnitQueue::new(
            *resource.resource_id(),
            session_id.clone(),
            3,
            8 * 1024 * 1024,
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
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("mrd-agent-dxgi-capture".to_owned())
            .spawn(move || {
                use mrd_capture_dxgi::DxgiDesktopCapture;
                use mrd_encode_openh264::OpenH264Encoder;
                use mrd_pipeline_core::{FrameCapture, VideoEncoder};

                let Ok(mut capture) = DxgiDesktopCapture::new_primary() else {
                    let _ = ready_tx.send(false);
                    return;
                };
                let Ok(mut encoder) =
                    OpenH264Encoder::new_speed(capture.width(), capture.height(), 60)
                else {
                    let _ = ready_tx.send(false);
                    return;
                };
                if ready_tx.send(true).is_err() {
                    return;
                }
                let mut sequence = 0_u64;
                while !thread_stop.load(Ordering::Acquire) {
                    let Ok(frame) = capture.capture_frame() else {
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
                        ) else {
                            return;
                        };
                        let accepted = queues.lock().ok().and_then(|mut all| {
                            all.get_mut(&resource_id).map(|queue| queue.push(unit))
                        });
                        if accepted != Some(true) {
                            return;
                        }
                    }
                }
            });
        let Ok(join) = join else {
            let _ = self.queues.lock().map(|mut all| all.remove(&resource_id));
            return false;
        };
        if !ready_rx.recv().unwrap_or(false) {
            stop.store(true, std::sync::atomic::Ordering::Release);
            let _ = join.join();
            let _ = self.queues.lock().map(|mut all| all.remove(&resource_id));
            return false;
        }
        self.workers.insert(
            resource_id,
            CaptureWorker {
                stop,
                join: Some(join),
            },
        );
        true
    }

    fn stop(&mut self, resource_id: &[u8; 16], _session_id: &SessionId) -> bool {
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
        let resources = self.workers.keys().copied().collect::<Vec<_>>();
        for resource_id in resources {
            let _ = self.stop(&resource_id, &SessionId(String::new()));
        }
    }
}

#[cfg(all(windows, test))]
mod tests {
    use super::{CaptureAdapter, WindowsDxgiOpenH264CaptureAdapter};
    use crate::media::{MediaResourceKind, MediaResourceRegistry};
    use mrd_proto::SessionId;

    #[test]
    #[ignore = "requires an interactive Windows desktop and display capture"]
    fn native_dxgi_capture_worker_starts_and_stops_without_leaking_a_resource() {
        let session = SessionId("capture-smoke".to_owned());
        let id = [0x41; 16];
        let mut registry = MediaResourceRegistry::new();
        registry.start(id, session.clone(), 0, MediaResourceKind::Capture, None);
        let resource = registry.get(&id).unwrap();
        let mut adapter = WindowsDxgiOpenH264CaptureAdapter::new();
        assert!(adapter.start(resource, &session));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(adapter.stop(&id, &session));
    }
}
