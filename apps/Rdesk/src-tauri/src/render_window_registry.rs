use std::collections::HashMap;

use mrd_proto::SessionId;
use serde::Serialize;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RenderWindowEntry {
    pub label: String,
    pub surface_id: String,
    pub role: String,
    pub renderer_attached: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RenderWindowContext {
    pub label: String,
    pub session_id: String,
    pub surface_id: String,
    pub role: String,
    pub renderer_attached: bool,
    pub session_window_count: usize,
}

#[derive(Default)]
pub struct RenderWindowRegistry {
    next_ids: HashMap<SessionId, u32>,
    windows_by_session: HashMap<SessionId, Vec<RenderWindowEntry>>,
}

impl RenderWindowRegistry {
    pub fn open_window<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        session_id: SessionId,
        surface_id: Option<String>,
    ) -> Result<String, String> {
        let next_id = self.next_ids.entry(session_id.clone()).or_insert(1);
        let label = format!("render-{}-{}", session_id.0, *next_id);
        let surface_id = surface_id.unwrap_or_else(|| format!("surface-{}", *next_id));
        *next_id += 1;

        let url = format!("/session/{}", session_id.0);
        WebviewWindowBuilder::new(app, label.clone(), WebviewUrl::App(url.into()))
            .title(format!("Remote Session {}", session_id.0))
            .decorations(false)
            .resizable(true)
            .inner_size(1280.0, 800.0)
            .build()
            .map_err(|error| format!("创建渲染窗口失败: {error}"))?;

        self.windows_by_session
            .entry(session_id)
            .or_default()
            .push(RenderWindowEntry {
                label: label.clone(),
                surface_id,
                role: "controller".to_string(),
                renderer_attached: false,
            });
        Ok(label)
    }

    pub fn list_window_contexts<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        session_id: &SessionId,
    ) -> Vec<RenderWindowContext> {
        let entries = self
            .windows_by_session
            .entry(session_id.clone())
            .or_default();
        entries.retain(|entry| app.get_webview_window(&entry.label).is_some());
        let count = entries.len();
        entries
            .iter()
            .map(|entry| RenderWindowContext {
                label: entry.label.clone(),
                session_id: session_id.0.clone(),
                surface_id: entry.surface_id.clone(),
                role: entry.role.clone(),
                renderer_attached: entry.renderer_attached,
                session_window_count: count,
            })
            .collect()
    }

    pub fn close_window<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        label: &str,
    ) -> Result<(), String> {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.close();
        }

        for entries in self.windows_by_session.values_mut() {
            entries.retain(|candidate| candidate.label != label);
        }

        Ok(())
    }

    pub fn set_renderer_attached<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        label: &str,
        renderer_attached: bool,
    ) {
        for entries in self.windows_by_session.values_mut() {
            entries.retain(|entry| app.get_webview_window(&entry.label).is_some());
            if let Some(entry) = entries.iter_mut().find(|entry| entry.label == label) {
                entry.renderer_attached = renderer_attached;
                return;
            }
        }
    }

    pub fn context_for_label<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        label: &str,
    ) -> Option<RenderWindowContext> {
        for (session_id, entries) in self.windows_by_session.iter_mut() {
            entries.retain(|entry| app.get_webview_window(&entry.label).is_some());
            if let Some(entry) = entries.iter().find(|entry| entry.label == label) {
                return Some(RenderWindowContext {
                    label: label.to_string(),
                    session_id: session_id.0.clone(),
                    surface_id: entry.surface_id.clone(),
                    role: entry.role.clone(),
                    renderer_attached: entry.renderer_attached,
                    session_window_count: entries.len(),
                });
            }
        }

        None
    }

    pub fn rebind_window_surface<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        label: &str,
        surface_id: String,
    ) -> Result<(SessionId, Option<String>), String> {
        for (session_id, entries) in self.windows_by_session.iter_mut() {
            entries.retain(|entry| app.get_webview_window(&entry.label).is_some());
            if let Some(entry) = entries.iter_mut().find(|entry| entry.label == label) {
                let previous_surface_id =
                    (entry.surface_id != surface_id).then(|| entry.surface_id.clone());
                entry.surface_id = surface_id;
                return Ok((session_id.clone(), previous_surface_id));
            }
        }

        Err(format!("未找到渲染窗口: {}", label))
    }

    pub fn surface_window_count<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        session_id: &SessionId,
        surface_id: &str,
    ) -> usize {
        let entries = self
            .windows_by_session
            .entry(session_id.clone())
            .or_default();
        entries.retain(|entry| app.get_webview_window(&entry.label).is_some());
        entries
            .iter()
            .filter(|entry| entry.surface_id == surface_id)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::{RenderWindowContext, RenderWindowEntry, RenderWindowRegistry};
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

    #[test]
    fn explicit_surface_id_can_be_reused_for_new_window() {
        let mut registry = RenderWindowRegistry::default();
        let session = SessionId("session-a".into());

        let next = registry.next_ids.entry(session.clone()).or_insert(1);
        let first_label = format!("render-{}-{}", session.0, *next);
        *next += 1;
        let second_label = format!("render-{}-{}", session.0, *next);

        registry.windows_by_session.insert(
            session.clone(),
            vec![
                RenderWindowEntry {
                    label: first_label,
                    surface_id: "surface-1".into(),
                    role: "controller".into(),
                    renderer_attached: true,
                },
                RenderWindowEntry {
                    label: second_label,
                    surface_id: "surface-1".into(),
                    role: "controller".into(),
                    renderer_attached: false,
                },
            ],
        );

        let entries = registry
            .windows_by_session
            .get(&session)
            .expect("session entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].surface_id, "surface-1");
        assert_eq!(entries[1].surface_id, "surface-1");
    }

    #[test]
    fn render_window_context_carries_label_and_session_counts() {
        let context = RenderWindowContext {
            label: "render-session-a-2".into(),
            session_id: "session-a".into(),
            surface_id: "surface-2".into(),
            role: "controller".into(),
            renderer_attached: true,
            session_window_count: 2,
        };

        assert_eq!(context.label, "render-session-a-2");
        assert_eq!(context.session_id, "session-a");
        assert_eq!(context.surface_id, "surface-2");
        assert_eq!(context.role, "controller");
        assert!(context.renderer_attached);
        assert_eq!(context.session_window_count, 2);
    }

    #[test]
    fn rebinding_window_surface_returns_previous_surface() {
        let mut registry = RenderWindowRegistry::default();
        let session = SessionId("session-a".into());
        registry.windows_by_session.insert(
            session.clone(),
            vec![RenderWindowEntry {
                label: "render-session-a-1".into(),
                surface_id: "surface-1".into(),
                role: "controller".into(),
                renderer_attached: true,
            }],
        );

        let entries = registry
            .windows_by_session
            .get_mut(&session)
            .expect("entries");
        let previous = if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.label == "render-session-a-1")
        {
            let previous = (entry.surface_id != "surface-2").then(|| entry.surface_id.clone());
            entry.surface_id = "surface-2".into();
            previous
        } else {
            None
        };

        assert_eq!(previous.as_deref(), Some("surface-1"));
        assert_eq!(entries[0].surface_id, "surface-2");
    }

    #[test]
    fn surface_window_count_counts_matching_entries() {
        let mut registry = RenderWindowRegistry::default();
        let session = SessionId("session-a".into());
        registry.windows_by_session.insert(
            session,
            vec![
                RenderWindowEntry {
                    label: "render-session-a-1".into(),
                    surface_id: "surface-1".into(),
                    role: "controller".into(),
                    renderer_attached: true,
                },
                RenderWindowEntry {
                    label: "render-session-a-2".into(),
                    surface_id: "surface-1".into(),
                    role: "controller".into(),
                    renderer_attached: false,
                },
                RenderWindowEntry {
                    label: "render-session-a-3".into(),
                    surface_id: "surface-2".into(),
                    role: "controller".into(),
                    renderer_attached: false,
                },
            ],
        );

        let count = registry
            .windows_by_session
            .get(&SessionId("session-a".into()))
            .expect("entries")
            .iter()
            .filter(|entry| entry.surface_id == "surface-1")
            .count();

        assert_eq!(count, 2);
    }
}
