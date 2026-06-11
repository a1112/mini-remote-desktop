use super::{create_platform_surface_renderer, surface_backend_matches_platform};
use mrd_ipc::AttachedRenderSurface;
use mrd_proto::SessionId;
use mrd_render::{BoxedRenderer, RenderFrame, RenderTarget};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

/// Native renderer instances owned by mrd-service for receiver sessions.
pub(crate) type SharedSurfaceRenderer = Arc<StdMutex<BoxedRenderer>>;

/// Native renderer instances owned by mrd-service for receiver sessions.
#[derive(Default)]
pub struct MediaSurfaceRendererRegistry {
    renderers: HashMap<(SessionId, String), SharedSurfaceRenderer>,
}

impl MediaSurfaceRendererRegistry {
    pub fn attach_surface(
        &mut self,
        session_id: &SessionId,
        surface: &AttachedRenderSurface,
    ) -> Result<(), String> {
        tracing::info!(
            session_id = %session_id.0,
            surface_id = %surface.surface_id,
            backend = %surface.backend,
            window_handle = ?surface.window_handle,
            "render-surface renderer attach requested"
        );
        if !surface_backend_matches_platform(&surface.backend) {
            tracing::info!(
                session_id = %session_id.0,
                surface_id = %surface.surface_id,
                backend = %surface.backend,
                "render-surface renderer attach skipped: backend does not match this platform"
            );
            return Ok(());
        }
        let key = (session_id.clone(), surface.surface_id.clone());
        if self.renderers.contains_key(&key) {
            tracing::info!(
                session_id = %session_id.0,
                surface_id = %surface.surface_id,
                renderer_count = self.session_surface_count(session_id),
                "render-surface renderer attach reused existing renderer"
            );
            return Ok(());
        }
        let window_handle = surface.window_handle.ok_or_else(|| {
            format!(
                "render surface {} is missing native handle",
                surface.surface_id
            )
        })?;
        let mut renderer = create_platform_surface_renderer(surface)?;
        renderer
            .attach_target(RenderTarget::WindowHandle(window_handle as isize))
            .map_err(|error| {
                format!("attach {} renderer target failed: {error}", surface.backend)
            })?;
        self.renderers
            .insert(key, Arc::new(StdMutex::new(renderer)));
        tracing::info!(
            session_id = %session_id.0,
            surface_id = %surface.surface_id,
            renderer_count = self.session_surface_count(session_id),
            "render-surface renderer attach completed"
        );
        Ok(())
    }

    pub fn detach_surface(&mut self, session_id: &SessionId, surface_id: &str) {
        let removed = self
            .renderers
            .remove(&(session_id.clone(), surface_id.to_string()))
            .is_some();
        tracing::info!(
            session_id = %session_id.0,
            surface_id = %surface_id,
            removed,
            renderer_count = self.session_surface_count(session_id),
            "render-surface renderer detach"
        );
    }

    pub fn detach_session(&mut self, session_id: &SessionId) {
        let before = self.renderers.len();
        self.renderers
            .retain(|(renderer_session_id, _), _| renderer_session_id != session_id);
        let removed = before.saturating_sub(self.renderers.len());
        tracing::info!(
            session_id = %session_id.0,
            removed,
            renderer_count = self.session_surface_count(session_id),
            "render-surface renderer detach session"
        );
    }

    pub fn renderers_for_session(&self, session_id: &SessionId) -> Vec<SharedSurfaceRenderer> {
        self.renderers
            .iter()
            .filter(|((renderer_session_id, _), _)| renderer_session_id == session_id)
            .map(|(_, renderer)| renderer.clone())
            .collect()
    }

    pub fn render_frame(
        &self,
        session_id: &SessionId,
        frame: &RenderFrame,
    ) -> Result<usize, String> {
        let mut rendered = 0;
        for renderer in self.renderers_for_session(session_id) {
            renderer
                .lock()
                .map_err(|_| "native surface renderer lock was poisoned".to_string())?
                .upload_frame(frame.clone())
                .map_err(|error| {
                    format!("upload frame to native surface renderer failed: {error}")
                })?;
            rendered += 1;
        }
        Ok(rendered)
    }

    pub fn session_surface_count(&self, session_id: &SessionId) -> usize {
        self.renderers
            .keys()
            .filter(|(renderer_session_id, _)| renderer_session_id == session_id)
            .count()
    }

    #[cfg(test)]
    pub fn insert_renderer_for_test(
        &mut self,
        session_id: &SessionId,
        surface_id: impl Into<String>,
        renderer: BoxedRenderer,
    ) {
        self.renderers.insert(
            (session_id.clone(), surface_id.into()),
            Arc::new(StdMutex::new(renderer)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::SessionId;
    use mrd_render::{RenderError, RenderFrame, RenderTarget, RendererInstance, RendererSnapshot};

    struct NoopRenderer;

    impl RendererInstance for NoopRenderer {
        fn attach_target(&mut self, _target: RenderTarget) -> Result<(), RenderError> {
            Ok(())
        }

        fn upload_frame(&mut self, _frame: RenderFrame) -> Result<(), RenderError> {
            Ok(())
        }

        fn snapshot(&self) -> RendererSnapshot {
            RendererSnapshot {
                attached_to_target: true,
                uploaded_frame_count: 0,
                presented_frame_count: 0,
                present_skipped_count: 0,
                render_queue_replacements: None,
                last_present_status: None,
                low_latency_frame_latency_target: None,
                swap_chain_max_frame_latency: None,
                swap_chain_allow_tearing: None,
                swap_chain_waitable_object: None,
                swap_chain_present_mode: None,
                display_refresh_hz: None,
                render_thread_priority: None,
                waitable_wait_count: None,
                waitable_wait_total_ms: None,
                waitable_timeout_count: None,
                last_waitable_wait_ms: None,
                last_render_prepare_wait_ms: None,
                last_render_shared_resource_ms: None,
                last_render_wait_for_drawable_ms: None,
                last_render_encode_commit_ms: None,
                last_render_draw_present_ms: None,
                last_width: 1,
                last_height: 1,
                last_pixel_format: None,
            }
        }
    }

    #[test]
    fn detach_session_removes_only_matching_surface_renderers() {
        let mut registry = MediaSurfaceRendererRegistry::default();
        let session_a = SessionId("surface-session-a".to_string());
        let session_b = SessionId("surface-session-b".to_string());

        registry.insert_renderer_for_test(&session_a, "surface-a", Box::new(NoopRenderer));
        registry.insert_renderer_for_test(&session_b, "surface-b", Box::new(NoopRenderer));

        registry.detach_session(&session_a);

        assert_eq!(registry.session_surface_count(&session_a), 0);
        assert_eq!(registry.session_surface_count(&session_b), 1);
    }
}
