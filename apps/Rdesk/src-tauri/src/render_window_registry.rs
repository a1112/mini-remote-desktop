use std::collections::HashMap;

use mrd_proto::SessionId;
use serde::Serialize;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

pub struct PendingRenderWindow {
    pub label: String,
    pub session_id: SessionId,
    pub surface_id: String,
    pub url: WebviewUrl,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RenderWindowEntry {
    pub label: String,
    pub surface_id: String,
    pub role: String,
    pub renderer_attached: bool,
    pub render_mode: String,
    pub native_surface_attached: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RenderWindowContext {
    pub label: String,
    pub session_id: String,
    pub surface_id: String,
    pub role: String,
    pub renderer_attached: bool,
    pub render_mode: String,
    pub native_surface_attached: bool,
    pub session_window_count: usize,
}

#[derive(Default)]
pub struct RenderWindowRegistry {
    next_ids: HashMap<SessionId, u32>,
    windows_by_session: HashMap<SessionId, Vec<RenderWindowEntry>>,
    native_surface_service_bindings: HashMap<String, NativeSurfaceServiceBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeSurfaceServiceBinding {
    backend: String,
    hwnd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSurfaceServiceAction {
    Attach,
    Detach,
    Unchanged,
}

impl RenderWindowRegistry {
    pub fn reserve_window(
        &mut self,
        session_id: SessionId,
        surface_id: Option<String>,
    ) -> Result<PendingRenderWindow, String> {
        let next_id = self.next_ids.entry(session_id.clone()).or_insert(1);
        let label = format!("render-{}-{}", session_id.0, *next_id);
        let surface_id = surface_id.unwrap_or_else(|| format!("surface-{}", *next_id));
        *next_id += 1;

        let url = remote_display_url(&session_id.0, &surface_id)?;
        Ok(PendingRenderWindow {
            label,
            session_id,
            surface_id,
            url,
        })
    }

    pub fn register_window(
        &mut self,
        session_id: SessionId,
        label: String,
        surface_id: String,
    ) -> usize {
        let entries = self.windows_by_session.entry(session_id).or_default();
        entries.push(RenderWindowEntry {
            label,
            surface_id,
            role: "controller".to_string(),
            renderer_attached: false,
            render_mode: "web".to_string(),
            native_surface_attached: false,
        });
        entries.len()
    }

    #[allow(dead_code)]
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

        let url = remote_display_url(&session_id.0, &surface_id)?;
        WebviewWindowBuilder::new(app, label.clone(), url)
            .title(format!("Rdesk Display {}", session_id.0))
            .decorations(false)
            .resizable(true)
            .inner_size(1280.0, 800.0)
            .min_inner_size(720.0, 420.0)
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
                render_mode: "web".to_string(),
                native_surface_attached: false,
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
                render_mode: entry.render_mode.clone(),
                native_surface_attached: entry.native_surface_attached,
                session_window_count: count,
            })
            .collect()
    }

    #[allow(dead_code)]
    pub fn close_window<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        label: &str,
    ) -> Result<(), String> {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.close();
        }

        self.remove_window_entry(label);

        Ok(())
    }

    pub fn remove_window_entry(&mut self, label: &str) -> Option<(SessionId, usize)> {
        let mut removed = None;

        for (session_id, entries) in self.windows_by_session.iter_mut() {
            let before = entries.len();
            entries.retain(|candidate| candidate.label != label);
            if entries.len() != before {
                removed = Some((session_id.clone(), entries.len()));
            }
        }

        self.windows_by_session
            .retain(|_session_id, entries| !entries.is_empty());
        if removed.is_some() {
            self.native_surface_service_bindings.remove(label);
        }
        removed
    }

    #[allow(dead_code)]
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
                    render_mode: entry.render_mode.clone(),
                    native_surface_attached: entry.native_surface_attached,
                    session_window_count: entries.len(),
                });
            }
        }

        None
    }

    #[allow(dead_code)]
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

    pub fn set_render_mode<R: tauri::Runtime>(
        &mut self,
        app: &tauri::AppHandle<R>,
        label: &str,
        render_mode: String,
        native_surface_attached: bool,
    ) -> Result<RenderWindowContext, String> {
        for (session_id, entries) in self.windows_by_session.iter_mut() {
            entries.retain(|entry| app.get_webview_window(&entry.label).is_some());
            let count = entries.len();
            if let Some(entry) = entries.iter_mut().find(|entry| entry.label == label) {
                entry.render_mode = render_mode;
                entry.native_surface_attached = native_surface_attached;
                entry.renderer_attached = native_surface_attached;
                return Ok(RenderWindowContext {
                    label: entry.label.clone(),
                    session_id: session_id.0.clone(),
                    surface_id: entry.surface_id.clone(),
                    role: entry.role.clone(),
                    renderer_attached: entry.renderer_attached,
                    render_mode: entry.render_mode.clone(),
                    native_surface_attached: entry.native_surface_attached,
                    session_window_count: count,
                });
            }
        }

        Err(format!("未找到渲染窗口: {}", label))
    }

    pub fn native_surface_service_action(
        &self,
        label: &str,
        attached: bool,
        backend: &str,
        hwnd: Option<&str>,
    ) -> NativeSurfaceServiceAction {
        if !attached {
            return if self.native_surface_service_bindings.contains_key(label) {
                NativeSurfaceServiceAction::Detach
            } else {
                NativeSurfaceServiceAction::Unchanged
            };
        }

        let next = NativeSurfaceServiceBinding {
            backend: backend.to_string(),
            hwnd: hwnd.map(str::to_string),
        };
        match self.native_surface_service_bindings.get(label) {
            Some(current) if current == &next => NativeSurfaceServiceAction::Unchanged,
            _ => NativeSurfaceServiceAction::Attach,
        }
    }

    pub fn record_native_surface_service_binding(
        &mut self,
        label: &str,
        attached: bool,
        backend: &str,
        hwnd: Option<&str>,
    ) {
        if attached {
            self.native_surface_service_bindings.insert(
                label.to_string(),
                NativeSurfaceServiceBinding {
                    backend: backend.to_string(),
                    hwnd: hwnd.map(str::to_string),
                },
            );
        } else {
            self.native_surface_service_bindings.remove(label);
        }
    }

    #[allow(dead_code)]
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

fn remote_display_url(session_id: &str, surface_id: &str) -> Result<WebviewUrl, String> {
    let path = format!("/display/{session_id}?surface={surface_id}");
    Ok(WebviewUrl::App(path.into()))
}

#[cfg(test)]
mod tests {
    use super::{
        NativeSurfaceServiceAction, RenderWindowContext, RenderWindowEntry, RenderWindowRegistry,
    };
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
                    render_mode: "d3d11_native".into(),
                    native_surface_attached: true,
                },
                RenderWindowEntry {
                    label: second_label,
                    surface_id: "surface-1".into(),
                    role: "controller".into(),
                    renderer_attached: false,
                    render_mode: "web".into(),
                    native_surface_attached: false,
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
            render_mode: "d3d11_native".into(),
            native_surface_attached: true,
            session_window_count: 2,
        };

        assert_eq!(context.label, "render-session-a-2");
        assert_eq!(context.session_id, "session-a");
        assert_eq!(context.surface_id, "surface-2");
        assert_eq!(context.role, "controller");
        assert!(context.renderer_attached);
        assert_eq!(context.render_mode, "d3d11_native");
        assert!(context.native_surface_attached);
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
                render_mode: "d3d11_native".into(),
                native_surface_attached: true,
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
                    render_mode: "d3d11_native".into(),
                    native_surface_attached: true,
                },
                RenderWindowEntry {
                    label: "render-session-a-2".into(),
                    surface_id: "surface-1".into(),
                    role: "controller".into(),
                    renderer_attached: false,
                    render_mode: "web".into(),
                    native_surface_attached: false,
                },
                RenderWindowEntry {
                    label: "render-session-a-3".into(),
                    surface_id: "surface-2".into(),
                    role: "controller".into(),
                    renderer_attached: false,
                    render_mode: "web".into(),
                    native_surface_attached: false,
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

    #[test]
    fn remove_window_entry_returns_session_and_remaining_count() {
        let mut registry = RenderWindowRegistry::default();
        let session = SessionId("session-a".into());
        registry.windows_by_session.insert(
            session.clone(),
            vec![
                RenderWindowEntry {
                    label: "render-session-a-1".into(),
                    surface_id: "surface-1".into(),
                    role: "controller".into(),
                    renderer_attached: true,
                    render_mode: "d3d11_native".into(),
                    native_surface_attached: true,
                },
                RenderWindowEntry {
                    label: "render-session-a-2".into(),
                    surface_id: "surface-2".into(),
                    role: "controller".into(),
                    renderer_attached: false,
                    render_mode: "web".into(),
                    native_surface_attached: false,
                },
            ],
        );

        let removed = registry.remove_window_entry("render-session-a-1");
        assert_eq!(removed, Some((session.clone(), 1)));
        assert_eq!(
            registry.remove_window_entry("render-session-a-2"),
            Some((session, 0))
        );
        assert!(registry.windows_by_session.is_empty());
    }

    #[test]
    fn native_surface_service_binding_skips_redundant_attach_for_same_handle() {
        let mut registry = RenderWindowRegistry::default();

        assert_eq!(
            registry.native_surface_service_action(
                "render-session-a-1",
                true,
                "d3d11",
                Some("0x1234")
            ),
            NativeSurfaceServiceAction::Attach
        );

        registry.record_native_surface_service_binding(
            "render-session-a-1",
            true,
            "d3d11",
            Some("0x1234"),
        );

        assert_eq!(
            registry.native_surface_service_action(
                "render-session-a-1",
                true,
                "d3d11",
                Some("0x1234")
            ),
            NativeSurfaceServiceAction::Unchanged
        );
    }

    #[test]
    fn native_surface_service_binding_reattaches_when_handle_changes() {
        let mut registry = RenderWindowRegistry::default();
        registry.record_native_surface_service_binding(
            "render-session-a-1",
            true,
            "d3d11",
            Some("0x1234"),
        );

        assert_eq!(
            registry.native_surface_service_action(
                "render-session-a-1",
                true,
                "d3d11",
                Some("0x5678")
            ),
            NativeSurfaceServiceAction::Attach
        );
    }

    #[test]
    fn native_surface_service_binding_detaches_once() {
        let mut registry = RenderWindowRegistry::default();
        registry.record_native_surface_service_binding(
            "render-session-a-1",
            true,
            "d3d11",
            Some("0x1234"),
        );

        assert_eq!(
            registry.native_surface_service_action("render-session-a-1", false, "web", None),
            NativeSurfaceServiceAction::Detach
        );
        registry.record_native_surface_service_binding("render-session-a-1", false, "web", None);
        assert_eq!(
            registry.native_surface_service_action("render-session-a-1", false, "web", None),
            NativeSurfaceServiceAction::Unchanged
        );
    }
}
