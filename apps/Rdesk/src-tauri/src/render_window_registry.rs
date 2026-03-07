use std::collections::HashMap;

use mrd_proto::SessionId;
use tauri::Manager;

#[derive(Default)]
pub struct RenderWindowRegistry {
    next_ids: HashMap<SessionId, u32>,
    windows_by_session: HashMap<SessionId, Vec<String>>,
}

impl RenderWindowRegistry {
    pub fn open_window<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        session_id: SessionId,
    ) -> Result<String, String> {
        let next_id = self.next_ids.entry(session_id.clone()).or_insert(1);
        let label = format!("render-{}-{}", session_id.0, *next_id);
        *next_id += 1;

        let url = format!("/session/{}", session_id.0);
        tauri::WindowBuilder::new(app, label.clone(), tauri::WindowUrl::App(url.into()))
            .title(format!("Remote Session {}", session_id.0))
            .decorations(false)
            .resizable(true)
            .inner_size(1280.0, 800.0)
            .build()
            .map_err(|error| format!("创建渲染窗口失败: {error}"))?;

        self.windows_by_session
            .entry(session_id)
            .or_default()
            .push(label.clone());
        Ok(label)
    }

    pub fn list_windows<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        session_id: &SessionId,
    ) -> Vec<String> {
        let labels = self.windows_by_session.entry(session_id.clone()).or_default();
        labels.retain(|label| app.get_window(label).is_some());
        labels.clone()
    }

    pub fn close_window<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        label: &str,
    ) -> Result<(), String> {
        if let Some(window) = app.get_window(label) {
            let _ = window.close();
        }

        for labels in self.windows_by_session.values_mut() {
            labels.retain(|candidate| candidate != label);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::RenderWindowRegistry;
    use mrd_proto::SessionId;

    #[test]
    fn allocating_windows_uses_monotonic_labels_per_session() {
        let mut registry = RenderWindowRegistry::default();
        let session = SessionId("session-a".into());

        let next = registry.next_ids.entry(session.clone()).or_insert(1);
        let first = format!("render-{}-{}", session.0, *next);
        *next += 1;
        let second = format!("render-{}-{}", session.0, *next);

        assert_eq!(first, "render-session-a-1");
        assert_eq!(second, "render-session-a-2");
    }
}
