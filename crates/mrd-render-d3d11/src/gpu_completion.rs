//! Retire frame leases independently of future frames, so a full decoder pool
//! can recover even when its producer temporarily stops emitting output.
use mrd_render::{GpuFrameLease, RenderError};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use windows::{
    core::ComInterface,
    Win32::{Foundation::BOOL, Graphics::Direct3D11::*},
};

const MAX_PENDING: usize = 8;

struct Pending {
    query: ID3D11Query,
    _lease: GpuFrameLease,
}

#[derive(Default)]
struct Queue {
    pending: VecDeque<Pending>,
    stopping: bool,
}

#[derive(Default)]
struct State {
    queue: Mutex<Queue>,
    wake: Condvar,
    count: AtomicUsize,
    failed: AtomicBool,
}

pub(super) struct GpuCompletionTracker {
    state: Arc<State>,
}

impl GpuCompletionTracker {
    pub(super) fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
    ) -> Result<Self, RenderError> {
        // GetData runs on the retirement worker while the render thread submits
        // draws. D3D11's explicit multithread protection serializes API access.
        let protected: ID3D11Multithread = context.cast().map_err(|error| {
            RenderError::Message(format!(
                "D3D11 lease completion requires multithread protection: {error}"
            ))
        })?;
        unsafe {
            protected.SetMultithreadProtected(true);
        }
        let state = Arc::new(State::default());
        let worker = state.clone();
        let device = device.clone();
        let context = context.clone();
        std::thread::Builder::new()
            .name("mrd-d3d11-retire".into())
            .spawn(move || {
                loop {
                    let pending = {
                        let mut queue = worker
                            .queue
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        while queue.pending.is_empty() && !queue.stopping {
                            queue = worker
                                .wake
                                .wait(queue)
                                .unwrap_or_else(|error| error.into_inner());
                        }
                        let Some(pending) = queue.pending.pop_front() else {
                            return;
                        };
                        pending
                    };
                    let started = Instant::now();
                    loop {
                        let mut done = BOOL(0);
                        let result = unsafe {
                            context.GetData(
                                &pending.query,
                                Some((&mut done as *mut BOOL).cast()),
                                std::mem::size_of::<BOOL>() as u32,
                                D3D11_ASYNC_GETDATA_DONOTFLUSH.0 as u32,
                            )
                        };
                        if (result.is_ok() && done.as_bool())
                            || unsafe { device.GetDeviceRemovedReason() }.is_err()
                        {
                            break;
                        }
                        if started.elapsed() >= Duration::from_secs(2) {
                            // Stop accepting frames, but keep the bounded pending
                            // leases alive until completion/device removal. Drop
                            // never waits on a stalled GPU or reuses live storage.
                            worker.failed.store(true, Ordering::Release);
                            std::thread::sleep(Duration::from_millis(20));
                        } else {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                    drop(pending);
                    worker.count.fetch_sub(1, Ordering::AcqRel);
                }
            })
            .map_err(|error| {
                RenderError::Message(format!(
                    "start D3D11 lease completion worker failed: {error}"
                ))
            })?;
        Ok(Self { state })
    }

    pub(super) fn can_submit(&self) -> Result<bool, RenderError> {
        if self.state.failed.load(Ordering::Acquire) {
            return Err(RenderError::Message(
                "D3D11 GPU frame completion timed out; pending frame storage remains retained"
                    .into(),
            ));
        }
        Ok(self.state.count.load(Ordering::Acquire) < MAX_PENDING)
    }

    // Only the owning render thread submits, after checking can_submit and
    // before another frame can be accepted.
    pub(super) fn submit(&self, query: ID3D11Query, lease: GpuFrameLease) {
        self.state.count.fetch_add(1, Ordering::AcqRel);
        let mut queue = self
            .state
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.pending.push_back(Pending {
            query,
            _lease: lease,
        });
        self.state.wake.notify_one();
    }
}

impl Drop for GpuCompletionTracker {
    fn drop(&mut self) {
        let mut queue = self
            .state
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.stopping = true;
        self.state.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirement_retains_storage_until_gpu_event_even_after_tracker_drop() {
        let renderer = crate::D3d11Renderer::new().unwrap();
        let tracker = GpuCompletionTracker::new(&renderer.device, &renderer.context).unwrap();
        let mut query = None;
        unsafe {
            renderer
                .device
                .CreateQuery(
                    &D3D11_QUERY_DESC {
                        Query: D3D11_QUERY_EVENT,
                        MiscFlags: 0,
                    },
                    Some(&mut query),
                )
                .unwrap();
        }
        let query = query.unwrap();
        let owner = Arc::new(42u32);
        tracker.submit(query.clone(), GpuFrameLease::from_arc(owner.clone()));
        drop(tracker);
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(
            Arc::strong_count(&owner),
            2,
            "uncompleted query must hold frame storage"
        );
        unsafe {
            renderer.context.End(&query);
            renderer.context.Flush();
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Arc::strong_count(&owner) > 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            Arc::strong_count(&owner),
            1,
            "completion must retire without another incoming frame"
        );
    }

    #[test]
    fn backpressure_is_bounded_and_completion_failure_is_explicit() {
        // Exercise admission separately from GPU timing. Only the render thread
        // increments count; the worker can only decrement it between calls.
        let tracker = GpuCompletionTracker {
            state: Arc::new(State::default()),
        };
        for pending in 0..MAX_PENDING {
            tracker.state.count.store(pending, Ordering::Release);
            assert!(tracker.can_submit().unwrap());
        }
        tracker.state.count.store(MAX_PENDING, Ordering::Release);
        assert!(!tracker.can_submit().unwrap());
        tracker.state.count.fetch_sub(1, Ordering::AcqRel);
        assert!(tracker.can_submit().unwrap());
        tracker.state.failed.store(true, Ordering::Release);
        let error = tracker.can_submit().unwrap_err().to_string();
        assert!(error.contains("timed out"));
        assert!(error.contains("retained"));
    }
}
