use mrd_proto::SessionId;
use std::collections::HashMap;
use tokio::task::{AbortHandle, Id};

/// Runtime media tasks keyed by session.
#[derive(Default)]
pub struct MediaTaskRegistry {
    tasks: HashMap<SessionId, Vec<AbortHandle>>,
}

impl MediaTaskRegistry {
    pub fn register(&mut self, session_id: SessionId, abort_handle: AbortHandle) {
        if abort_handle.is_finished() {
            return;
        }
        self.tasks.entry(session_id).or_default().push(abort_handle);
    }

    pub fn forget_task(&mut self, session_id: &SessionId, task_id: Id) -> bool {
        let Some(handles) = self.tasks.get_mut(session_id) else {
            return false;
        };
        let original_len = handles.len();
        handles.retain(|handle| handle.id() != task_id);
        let removed = handles.len() != original_len;
        if handles.is_empty() {
            self.tasks.remove(session_id);
        }
        removed
    }

    pub fn abort_session(&mut self, session_id: &SessionId) -> usize {
        let handles = self.tasks.remove(session_id).unwrap_or_default();
        let count = handles.len();
        for handle in handles {
            handle.abort();
        }
        count
    }

    pub fn active_count(&self, session_id: &SessionId) -> usize {
        self.tasks.get(session_id).map_or(0, Vec::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::SessionId;

    #[tokio::test]
    async fn abort_session_aborts_all_registered_tasks_and_clears_session() {
        let session_id = SessionId("media-session".to_string());
        let mut registry = MediaTaskRegistry::default();
        let first = tokio::spawn(async { std::future::pending::<()>().await });
        let second = tokio::spawn(async { std::future::pending::<()>().await });

        registry.register(session_id.clone(), first.abort_handle());
        registry.register(session_id.clone(), second.abort_handle());

        assert_eq!(registry.active_count(&session_id), 2);
        assert_eq!(registry.abort_session(&session_id), 2);
        assert_eq!(registry.active_count(&session_id), 0);

        assert!(first.await.unwrap_err().is_cancelled());
        assert!(second.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn completed_tasks_are_not_registered_and_forget_task_preserves_other_handles() {
        let session_id = SessionId("completed-media-session".to_string());
        let mut registry = MediaTaskRegistry::default();
        let already_finished = tokio::spawn(async {});
        let already_finished_handle = already_finished.abort_handle();
        already_finished.await.expect("second completed task");

        registry.register(session_id.clone(), already_finished_handle);
        assert_eq!(registry.active_count(&session_id), 0);

        let forgotten = tokio::spawn(async { std::future::pending::<()>().await });
        let retained = tokio::spawn(async { std::future::pending::<()>().await });
        registry.register(session_id.clone(), forgotten.abort_handle());
        registry.register(session_id.clone(), retained.abort_handle());
        assert_eq!(registry.active_count(&session_id), 2);
        assert!(registry.forget_task(&session_id, forgotten.id()));
        assert_eq!(registry.active_count(&session_id), 1);
        assert!(!forgotten.is_finished());
        assert!(!retained.is_finished());

        assert_eq!(registry.abort_session(&session_id), 1);
        assert!(retained.await.unwrap_err().is_cancelled());

        forgotten.abort();
        assert!(forgotten.await.unwrap_err().is_cancelled());
    }
}
