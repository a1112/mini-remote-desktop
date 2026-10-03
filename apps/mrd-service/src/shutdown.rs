//! Runtime-owned shutdown admission and IPC acknowledgement fence.

use mrd_ipc::ShutdownMode;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct State {
    runtime_bound: bool,
    requested: Option<ShutdownMode>,
    acknowledged: Option<ShutdownMode>,
    admissions: usize,
    admission_epoch: u64,
}

#[derive(Default)]
pub struct ShutdownCoordinator {
    state: Mutex<State>,
    changed: Notify,
}

fn priority(mode: &ShutdownMode) -> u8 {
    match mode {
        ShutdownMode::AfterSessions => 0,
        ShutdownMode::Graceful => 1,
        ShutdownMode::Force => 2,
    }
}

impl ShutdownCoordinator {
    pub fn bind_runtime(self: &Arc<Self>) -> anyhow::Result<RuntimeBinding> {
        let mut state = self.state.lock().unwrap();
        anyhow::ensure!(!state.runtime_bound, "shutdown runtime is already bound");
        state.runtime_bound = true;
        Ok(RuntimeBinding(Arc::clone(self)))
    }

    /// Holds admission until a session-start request has finished projecting state.
    pub fn admit(self: &Arc<Self>) -> anyhow::Result<AdmissionPermit> {
        let mut state = self.state.lock().unwrap();
        anyhow::ensure!(state.requested.is_none(), "service is shutting down");
        state.admissions += 1;
        state.admission_epoch = state.admission_epoch.wrapping_add(1);
        Ok(AdmissionPermit(Arc::clone(self)))
    }

    pub fn request(&self, mode: ShutdownMode) -> anyhow::Result<()> {
        let mut state = self.state.lock().unwrap();
        anyhow::ensure!(
            state.runtime_bound,
            "service shutdown runtime is unavailable"
        );
        if state
            .requested
            .as_ref()
            .is_none_or(|current| priority(&mode) > priority(current))
        {
            state.requested = Some(mode);
        }
        self.changed.notify_one();
        Ok(())
    }

    /// Called only after the local IPC transport has attempted to flush the Ack.
    pub fn acknowledge(&self, mode: ShutdownMode) {
        let mut state = self.state.lock().unwrap();
        if state
            .acknowledged
            .as_ref()
            .is_none_or(|current| priority(&mode) > priority(current))
        {
            state.acknowledged = Some(mode);
        }
        self.changed.notify_one();
    }

    pub fn admission_epoch(&self) -> u64 {
        self.state.lock().unwrap().admission_epoch
    }

    pub fn is_requested(&self) -> bool {
        self.state.lock().unwrap().requested.is_some()
    }

    pub fn ready_mode_at_epoch(
        &self,
        active_sessions: usize,
        admission_epoch: u64,
    ) -> Option<ShutdownMode> {
        let state = self.state.lock().unwrap();
        let mode = state.requested.as_ref()?;
        if !state.runtime_bound
            || state
                .acknowledged
                .as_ref()
                .is_none_or(|ack| priority(ack) < priority(mode))
        {
            return None;
        }
        if *mode == ShutdownMode::AfterSessions
            && (active_sessions > 0
                || state.admissions > 0
                || state.admission_epoch != admission_epoch)
        {
            return None;
        }
        Some(mode.clone())
    }

    pub async fn changed(&self) {
        self.changed.notified().await;
    }
}

pub struct RuntimeBinding(Arc<ShutdownCoordinator>);
impl Drop for RuntimeBinding {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().runtime_bound = false;
        self.0.changed.notify_one();
    }
}

pub struct AdmissionPermit(Arc<ShutdownCoordinator>);
impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap();
        state.admissions -= 1;
        state.admission_epoch = state.admission_epoch.wrapping_add(1);
        self.0.changed.notify_one();
    }
}

pub fn cleanup_timeout(mode: &ShutdownMode) -> Duration {
    if *mode == ShutdownMode::Force {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(20)
    }
}

/// Includes coordinator-owned WAN workflows that have no public projection yet.
pub async fn active_session_count(app_state: &crate::AppState) -> usize {
    use mrd_application::ports::SessionLifecycleState;
    let mut ids = app_state
        .sessions
        .lock()
        .await
        .list_all()
        .into_iter()
        .filter(|session| {
            !matches!(
                session.lifecycle_state,
                SessionLifecycleState::Closed | SessionLifecycleState::Failed { .. }
            )
        })
        .map(|session| session.session_id)
        .collect::<std::collections::HashSet<_>>();
    if let Some(coordinator) = app_state.wan_session_coordinator() {
        for session in coordinator.snapshots().await {
            if !session.phase().is_terminal() {
                ids.insert(session.identity().session_id().clone());
            }
        }
    }
    ids.len()
}

pub async fn wait_for_shutdown(app_state: &crate::AppState) -> ShutdownMode {
    loop {
        let epoch = app_state.shutdown.admission_epoch();
        // Immediate shutdown must remain able to preempt an awaited registry lock.
        if let Some(mode) = app_state.shutdown.ready_mode_at_epoch(usize::MAX, epoch) {
            return mode;
        }
        let count = tokio::select! {
            count = active_session_count(app_state) => count,
            _ = app_state.shutdown.changed() => continue,
        };
        if let Some(mode) = app_state.shutdown.ready_mode_at_epoch(count, epoch) {
            return mode;
        }
        tokio::select! {
            _ = app_state.shutdown.changed() => {},
            _ = tokio::time::sleep(Duration::from_millis(250)) => {},
        }
    }
}
