use mrd_proto::SessionId;
use std::collections::HashMap;
use tokio::task::AbortHandle;

/// Runtime media tasks keyed by session.
#[derive(Default)]
pub struct MediaTaskRegistry {
    tasks: HashMap<SessionId, Vec<AbortHandle>>,
}

impl MediaTaskRegistry {
    pub fn register(&mut self, session_id: SessionId, abort_handle: AbortHandle) {
        self.tasks.entry(session_id).or_default().push(abort_handle);
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
}
