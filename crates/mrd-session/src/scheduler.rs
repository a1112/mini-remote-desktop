//! Multi-session scheduler and resource isolation.
//!
//! The scheduler limits concurrent streaming sessions and keeps session-level
//! resource metadata independent of concrete transport implementations.

use std::collections::{hash_map::Entry, HashMap};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::{SessionId, SessionLifecycleState};

/// Session priority for scheduling decisions
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SessionPriority {
    /// Background or best-effort sessions.
    Low = 0,
    /// Normal interactive sessions.
    #[default]
    Normal = 1,
    /// Preferred sessions when resources are constrained.
    High = 2,
    /// Sessions that should be admitted before all others.
    Critical = 3,
}

/// Resource limits for a session
#[derive(Debug, Clone, Default)]
pub struct SessionResourceLimits {
    /// Maximum bitrate in Mbps
    pub max_bitrate_mbps: Option<f32>,
    /// Maximum frame rate
    pub max_fps: Option<u32>,
    /// Maximum resolution (width, height)
    pub max_resolution: Option<(u32, u32)>,
    /// CPU budget percentage (0-100)
    pub cpu_budget_percent: Option<u32>,
}

/// Session entry with scheduling metadata
#[derive(Debug)]
struct ScheduledSession {
    /// Unique registration generation used to reject waiters from a removed session.
    generation: u64,
    /// Current lifecycle state
    state: SessionLifecycleState,
    /// Session priority
    priority: SessionPriority,
    /// Resource limits
    limits: SessionResourceLimits,
    /// Last activity timestamp
    last_activity: std::time::Instant,
    /// Whether this session is currently active (streaming)
    is_active: bool,
}

impl ScheduledSession {
    fn new(generation: u64, priority: SessionPriority, limits: SessionResourceLimits) -> Self {
        let now = std::time::Instant::now();
        Self {
            generation,
            state: SessionLifecycleState::Created,
            priority,
            limits,
            last_activity: now,
            is_active: false,
        }
    }

    /// Check if session should be considered idle
    fn is_idle(&self, timeout: Duration) -> bool {
        !self.is_active && self.last_activity.elapsed() > timeout
    }

    /// Update activity timestamp
    fn touch(&mut self) {
        self.last_activity = std::time::Instant::now();
    }
}

/// Multi-session scheduler with resource isolation
#[derive(Debug)]
pub struct SessionScheduler {
    /// All scheduled sessions
    sessions: Arc<Mutex<HashMap<SessionId, ScheduledSession>>>,
    /// Semaphore for concurrent streaming sessions
    streaming_semaphore: Arc<Semaphore>,
    /// Monotonically increasing token for each session registration.
    next_session_generation: AtomicU64,
    /// Maximum concurrent streaming sessions
    max_concurrent_streams: usize,
    /// Session timeout for idle sessions
    idle_timeout: Duration,
}

impl SessionScheduler {
    /// Create a new session scheduler
    pub fn new(max_concurrent_streams: usize, idle_timeout: Duration) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            streaming_semaphore: Arc::new(Semaphore::new(max_concurrent_streams)),
            next_session_generation: AtomicU64::new(0),
            max_concurrent_streams,
            idle_timeout,
        }
    }

    /// Register a new session for scheduling
    pub async fn register_session(
        &self,
        session_id: SessionId,
        priority: SessionPriority,
        limits: SessionResourceLimits,
    ) -> Result<(), SessionSchedulerError> {
        let mut sessions = self.sessions.lock().await;
        match sessions.entry(session_id) {
            Entry::Vacant(entry) => {
                let generation = self.next_session_generation.fetch_add(1, Ordering::Relaxed);
                entry.insert(ScheduledSession::new(generation, priority, limits));
                Ok(())
            }
            Entry::Occupied(_) => Err(SessionSchedulerError::SessionAlreadyExists),
        }
    }

    /// Unregister a session
    pub async fn unregister_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), SessionSchedulerError> {
        let mut sessions = self.sessions.lock().await;
        sessions
            .remove(session_id)
            .ok_or(SessionSchedulerError::SessionNotFound)?;
        Ok(())
    }

    /// Request to start streaming for a session
    ///
    /// Returns a permit that must be held while streaming. When dropped,
    /// the permit is released allowing another session to stream.
    pub async fn acquire_streaming_permit(
        &self,
        session_id: &SessionId,
    ) -> Result<StreamingPermit, SessionSchedulerError> {
        let generation = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(session_id)
                .ok_or(SessionSchedulerError::SessionNotFound)?;
            session.touch();
            session.generation
        };

        // Acquire semaphore permit (this waits if max concurrent reached)
        // Convert to owned permit for storage
        let permit = self
            .streaming_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SessionSchedulerError::SchedulerClosed)?;

        // The session could have been unregistered while this request waited for a
        // semaphore permit. Revalidate its registration before handing out the permit.
        {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(session_id)
                .filter(|session| session.generation == generation)
                .ok_or(SessionSchedulerError::SessionNotFound)?;
            session.is_active = true;
            session.state = SessionLifecycleState::Streaming;
        }

        Ok(StreamingPermit {
            session_id: session_id.clone(),
            generation,
            sessions: self.sessions.clone(),
            permit,
        })
    }

    /// Update session state
    pub async fn update_session_state(
        &self,
        session_id: &SessionId,
        state: SessionLifecycleState,
    ) -> Result<(), SessionSchedulerError> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or(SessionSchedulerError::SessionNotFound)?;
        session.state = state.clone();
        session.touch();

        // Update active flag based on state
        session.is_active = matches!(state, SessionLifecycleState::Streaming);

        Ok(())
    }

    /// Get session priority
    pub async fn get_session_priority(&self, session_id: &SessionId) -> Option<SessionPriority> {
        let sessions = self.sessions.lock().await;
        sessions.get(session_id).map(|s| s.priority)
    }

    /// Set session priority
    pub async fn set_session_priority(
        &self,
        session_id: &SessionId,
        priority: SessionPriority,
    ) -> Result<(), SessionSchedulerError> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or(SessionSchedulerError::SessionNotFound)?;
        session.priority = priority;
        Ok(())
    }

    /// Get session resource limits
    pub async fn get_session_limits(
        &self,
        session_id: &SessionId,
    ) -> Option<SessionResourceLimits> {
        let sessions = self.sessions.lock().await;
        sessions.get(session_id).map(|s| s.limits.clone())
    }

    /// Set session resource limits
    pub async fn set_session_limits(
        &self,
        session_id: &SessionId,
        limits: SessionResourceLimits,
    ) -> Result<(), SessionSchedulerError> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(session_id)
            .ok_or(SessionSchedulerError::SessionNotFound)?;
        session.limits = limits;
        Ok(())
    }

    /// Get all active session IDs
    pub async fn active_sessions(&self) -> Vec<SessionId> {
        let sessions = self.sessions.lock().await;
        sessions
            .iter()
            .filter(|(_, s)| s.is_active)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get count of currently streaming sessions
    pub async fn streaming_count(&self) -> usize {
        self.sessions
            .lock()
            .await
            .values()
            .filter(|s| s.is_active)
            .count()
    }

    /// Get maximum concurrent streaming sessions
    pub fn max_concurrent_streams(&self) -> usize {
        self.max_concurrent_streams
    }

    /// Get available streaming slots
    pub async fn available_slots(&self) -> usize {
        self.streaming_semaphore.available_permits()
    }

    /// Prune idle sessions that have exceeded the timeout
    pub async fn prune_idle_sessions(&self) -> Vec<SessionId> {
        let mut sessions = self.sessions.lock().await;
        let mut to_remove = Vec::new();

        for (id, session) in sessions.iter() {
            if session.is_idle(self.idle_timeout) {
                to_remove.push(id.clone());
            }
        }

        for id in &to_remove {
            sessions.remove(id);
        }

        to_remove
    }

    /// Get scheduler statistics
    pub async fn stats(&self) -> SessionSchedulerStats {
        let sessions = self.sessions.lock().await;
        let active_count = sessions.values().filter(|s| s.is_active).count();
        let total_count = sessions.len();

        SessionSchedulerStats {
            total_sessions: total_count,
            active_sessions: active_count,
            available_slots: self.streaming_semaphore.available_permits(),
            max_concurrent_streams: self.max_concurrent_streams,
        }
    }
}

/// Permit for streaming - releases semaphore when dropped
pub struct StreamingPermit {
    session_id: SessionId,
    generation: u64,
    sessions: Arc<Mutex<HashMap<SessionId, ScheduledSession>>>,
    #[allow(dead_code)]
    permit: OwnedSemaphorePermit,
}

impl StreamingPermit {
    /// Get the session ID this permit is for
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
}

impl Drop for StreamingPermit {
    fn drop(&mut self) {
        // Try to mark session as inactive (best effort)
        // The OwnedSemaphorePermit will automatically release the semaphore permit
        if let Ok(mut sessions) = self.sessions.try_lock() {
            if let Some(session) = sessions.get_mut(&self.session_id) {
                if session.generation == self.generation {
                    session.is_active = false;
                }
            }
        }
    }
}

/// Session scheduler errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSchedulerError {
    /// Requested session does not exist.
    SessionNotFound,
    /// A session with the same id is already registered.
    SessionAlreadyExists,
    /// Scheduler has been shut down.
    SchedulerClosed,
    /// No streaming slots are currently available.
    MaxConcurrentSessionsReached,
    /// Requested transition is not valid for the current state.
    InvalidState,
}

impl std::fmt::Display for SessionSchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionNotFound => write!(f, "Session not found"),
            Self::SessionAlreadyExists => write!(f, "Session already exists"),
            Self::SchedulerClosed => write!(f, "Scheduler closed"),
            Self::MaxConcurrentSessionsReached => write!(f, "Max concurrent sessions reached"),
            Self::InvalidState => write!(f, "Invalid session state"),
        }
    }
}

impl std::error::Error for SessionSchedulerError {}

/// Session scheduler statistics
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSchedulerStats {
    /// Number of registered sessions.
    pub total_sessions: usize,
    /// Number of sessions currently holding a streaming permit.
    pub active_sessions: usize,
    /// Number of streaming permits still available.
    pub available_slots: usize,
    /// Maximum concurrent streaming sessions configured for the scheduler.
    pub max_concurrent_streams: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scheduler_registers_new_session() {
        let scheduler = SessionScheduler::new(2, Duration::from_secs(30));

        let session_id = SessionId("test-session-1".to_string());
        scheduler
            .register_session(
                session_id.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        let priority = scheduler.get_session_priority(&session_id).await;
        assert_eq!(priority, Some(SessionPriority::Normal));
    }

    #[tokio::test]
    async fn scheduler_prevents_duplicate_session_registration() {
        let scheduler = SessionScheduler::new(2, Duration::from_secs(30));

        let session_id = SessionId("test-session-dup".to_string());
        scheduler
            .register_session(
                session_id.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        let result = scheduler
            .register_session(
                session_id,
                SessionPriority::High,
                SessionResourceLimits::default(),
            )
            .await;

        assert_eq!(result, Err(SessionSchedulerError::SessionAlreadyExists));
    }

    #[tokio::test]
    async fn scheduler_acquires_streaming_permit() {
        let scheduler = SessionScheduler::new(2, Duration::from_secs(30));

        let session_id = SessionId("test-session-permit".to_string());
        scheduler
            .register_session(
                session_id.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        let permit = scheduler
            .acquire_streaming_permit(&session_id)
            .await
            .unwrap();
        assert_eq!(permit.session_id(), &session_id);

        // Permit is held while in scope
        assert_eq!(scheduler.streaming_count().await, 1);

        // Permit released when dropped
        drop(permit);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(scheduler.streaming_count().await, 0);
    }

    #[tokio::test]
    async fn scheduler_respects_max_concurrent_streams() {
        let scheduler = SessionScheduler::new(2, Duration::from_secs(30));

        let session1 = SessionId("session-1".to_string());
        let session2 = SessionId("session-2".to_string());
        let session3 = SessionId("session-3".to_string());

        for session in [&session1, &session2, &session3] {
            scheduler
                .register_session(
                    session.clone(),
                    SessionPriority::Normal,
                    SessionResourceLimits::default(),
                )
                .await
                .unwrap();
        }

        // Acquire permits for session 1 and 2
        let _permit1 = scheduler.acquire_streaming_permit(&session1).await.unwrap();
        let _permit2 = scheduler.acquire_streaming_permit(&session2).await.unwrap();

        // Session 3 should still be able to acquire (semaphore waits, not errors)
        let permit3 = tokio::time::timeout(
            Duration::from_millis(100),
            scheduler.acquire_streaming_permit(&session3),
        )
        .await;

        // Should timeout because max concurrent reached
        assert!(permit3.is_err());

        // Release permit 1
        drop(_permit1);
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Now session 3 should acquire
        let _permit3 = scheduler.acquire_streaming_permit(&session3).await.unwrap();
        assert_eq!(scheduler.streaming_count().await, 2);
    }

    #[tokio::test]
    async fn scheduler_updates_session_state() {
        let scheduler = SessionScheduler::new(2, Duration::from_secs(30));

        let session_id = SessionId("test-state".to_string());
        scheduler
            .register_session(
                session_id.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        scheduler
            .update_session_state(&session_id, SessionLifecycleState::Connected)
            .await
            .unwrap();

        let sessions = scheduler.sessions.lock().await;
        assert_eq!(
            sessions[&session_id].state,
            SessionLifecycleState::Connected
        );
    }

    #[tokio::test]
    async fn scheduler_sets_session_priority() {
        let scheduler = SessionScheduler::new(2, Duration::from_secs(30));

        let session_id = SessionId("test-priority".to_string());
        scheduler
            .register_session(
                session_id.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        scheduler
            .set_session_priority(&session_id, SessionPriority::High)
            .await
            .unwrap();

        let priority = scheduler.get_session_priority(&session_id).await;
        assert_eq!(priority, Some(SessionPriority::High));
    }

    #[tokio::test]
    async fn scheduler_sets_resource_limits() {
        let scheduler = SessionScheduler::new(2, Duration::from_secs(30));

        let session_id = SessionId("test-limits".to_string());
        scheduler
            .register_session(
                session_id.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        let limits = SessionResourceLimits {
            max_bitrate_mbps: Some(10.0),
            max_fps: Some(30),
            max_resolution: Some((1920, 1080)),
            cpu_budget_percent: Some(50),
        };

        scheduler
            .set_session_limits(&session_id, limits.clone())
            .await
            .unwrap();

        let retrieved = scheduler.get_session_limits(&session_id).await.unwrap();
        assert_eq!(retrieved.max_bitrate_mbps, limits.max_bitrate_mbps);
        assert_eq!(retrieved.max_fps, limits.max_fps);
        assert_eq!(retrieved.max_resolution, limits.max_resolution);
        assert_eq!(retrieved.cpu_budget_percent, limits.cpu_budget_percent);
    }

    #[tokio::test]
    async fn scheduler_prunes_idle_sessions() {
        let scheduler = SessionScheduler::new(2, Duration::from_millis(50));

        let session1 = SessionId("idle-session-1".to_string());
        let session2 = SessionId("idle-session-2".to_string());

        scheduler
            .register_session(
                session1.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        scheduler
            .register_session(
                session2.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        // Acquire and immediately release permit for session 1 (makes it active then inactive)
        {
            let _permit = scheduler.acquire_streaming_permit(&session1).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Session 2 stays inactive

        // Wait for idle timeout
        tokio::time::sleep(Duration::from_millis(60)).await;

        let pruned = scheduler.prune_idle_sessions().await;
        assert!(pruned.contains(&session1));
        assert!(pruned.contains(&session2));

        // Verify sessions are removed
        assert!(scheduler.get_session_priority(&session1).await.is_none());
        assert!(scheduler.get_session_priority(&session2).await.is_none());
    }

    #[tokio::test]
    async fn scheduler_reports_stats() {
        let scheduler = SessionScheduler::new(3, Duration::from_secs(30));

        let session1 = SessionId("stats-1".to_string());
        let session2 = SessionId("stats-2".to_string());

        scheduler
            .register_session(
                session1.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        scheduler
            .register_session(
                session2.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        let stats = scheduler.stats().await;
        assert_eq!(stats.total_sessions, 2);
        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.available_slots, 3);

        // Start streaming for session 1
        let _permit = scheduler.acquire_streaming_permit(&session1).await.unwrap();

        let stats = scheduler.stats().await;
        assert_eq!(stats.active_sessions, 1);
        assert_eq!(stats.available_slots, 2);
    }

    #[tokio::test]
    async fn scheduler_unregisters_session() {
        let scheduler = SessionScheduler::new(2, Duration::from_secs(30));

        let session_id = SessionId("unregister-me".to_string());
        scheduler
            .register_session(
                session_id.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();

        scheduler.unregister_session(&session_id).await.unwrap();

        let priority = scheduler.get_session_priority(&session_id).await;
        assert!(priority.is_none());
    }

    #[tokio::test]
    async fn scheduler_rejects_waiter_after_session_is_unregistered_and_replaced() {
        let scheduler = Arc::new(SessionScheduler::new(1, Duration::from_secs(30)));
        let holder_session = SessionId("holder-session".to_string());
        let waiting_session = SessionId("waiting-session".to_string());

        for session in [&holder_session, &waiting_session] {
            scheduler
                .register_session(
                    session.clone(),
                    SessionPriority::Normal,
                    SessionResourceLimits::default(),
                )
                .await
                .unwrap();
        }

        let holder = scheduler
            .acquire_streaming_permit(&holder_session)
            .await
            .unwrap();
        {
            let mut sessions = scheduler.sessions.lock().await;
            sessions
                .get_mut(&waiting_session)
                .expect("registered waiting session")
                .last_activity = std::time::Instant::now() - Duration::from_secs(1);
        }
        let waiter_scheduler = scheduler.clone();
        let waiter_session = waiting_session.clone();
        let waiter = tokio::spawn(async move {
            waiter_scheduler
                .acquire_streaming_permit(&waiter_session)
                .await
        });

        loop {
            let waiter_started = scheduler
                .sessions
                .lock()
                .await
                .get(&waiting_session)
                .expect("waiting session remains registered")
                .last_activity
                .elapsed()
                < Duration::from_millis(100);
            if waiter_started {
                break;
            }
            tokio::task::yield_now().await;
        }
        scheduler
            .unregister_session(&waiting_session)
            .await
            .unwrap();
        scheduler
            .register_session(
                waiting_session.clone(),
                SessionPriority::Normal,
                SessionResourceLimits::default(),
            )
            .await
            .unwrap();
        drop(holder);

        let result = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter should wake")
            .expect("waiter task should not panic");
        assert!(matches!(
            result,
            Err(SessionSchedulerError::SessionNotFound)
        ));
        assert_eq!(scheduler.available_slots().await, 1);
    }

    #[tokio::test]
    async fn scheduler_lists_active_sessions() {
        let scheduler = SessionScheduler::new(3, Duration::from_secs(30));

        let session1 = SessionId("active-1".to_string());
        let session2 = SessionId("active-2".to_string());
        let session3 = SessionId("active-3".to_string());

        for session in [&session1, &session2, &session3] {
            scheduler
                .register_session(
                    session.clone(),
                    SessionPriority::Normal,
                    SessionResourceLimits::default(),
                )
                .await
                .unwrap();
        }

        let _permit1 = scheduler.acquire_streaming_permit(&session1).await.unwrap();
        let _permit2 = scheduler.acquire_streaming_permit(&session2).await.unwrap();

        let active = scheduler.active_sessions().await;
        assert_eq!(active.len(), 2);
        assert!(active.contains(&session1));
        assert!(active.contains(&session2));
        assert!(!active.contains(&session3));
    }
}
