use std::collections::HashMap;

use mrd_proto::SessionId;
use serde::Serialize;

use crate::render_surface_catalog::{RenderSurfaceCatalog, RenderSurfaceDescriptor};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SurfaceSourceBinding {
    pub surface_id: String,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionLifecycleSnapshot {
    pub session_id: String,
    pub current_surface_id: Option<String>,
    pub surfaces: Vec<RenderSurfaceDescriptor>,
    pub available_source_ids: Vec<String>,
    pub surface_source_bindings: Vec<SurfaceSourceBinding>,
}

#[derive(Default)]
pub struct SessionLifecycleCoordinator {
    surfaces: RenderSurfaceCatalog,
    available_sources_by_session: HashMap<SessionId, Vec<String>>,
    bindings_by_session: HashMap<SessionId, HashMap<String, String>>,
}

impl SessionLifecycleCoordinator {
    pub fn create_surface(
        &mut self,
        session_id: SessionId,
        name: Option<String>,
    ) -> RenderSurfaceDescriptor {
        self.surfaces.create_surface(session_id, name)
    }

    pub fn ensure_surface(
        &mut self,
        session_id: SessionId,
        surface_id: String,
    ) -> RenderSurfaceDescriptor {
        self.surfaces.ensure_surface(session_id, surface_id)
    }

    pub fn list_surfaces(&self, session_id: &SessionId) -> Vec<RenderSurfaceDescriptor> {
        self.surfaces.list_surfaces(session_id)
    }

    pub fn select_current_surface(
        &mut self,
        session_id: SessionId,
        surface_id: String,
    ) -> Result<(), String> {
        self.surfaces.select_current_surface(session_id, surface_id)
    }

    pub fn current_surface_id(&self, session_id: &SessionId) -> Option<String> {
        self.surfaces.current_surface_id(session_id)
    }

    pub fn update_available_sources(
        &mut self,
        session_id: SessionId,
        available_source_ids: Vec<String>,
    ) {
        self.available_sources_by_session
            .insert(session_id.clone(), available_source_ids.clone());
        if let Some(bindings) = self.bindings_by_session.get_mut(&session_id) {
            bindings.retain(|_, source_id| available_source_ids.contains(source_id));
        }
    }

    pub fn bind_surface_source(
        &mut self,
        session_id: SessionId,
        surface_id: String,
        source_id: String,
    ) -> Result<(), String> {
        let available_sources = self
            .available_sources_by_session
            .get(&session_id)
            .ok_or_else(|| format!("未找到会话 source 列表: {}", session_id.0))?;
        if !available_sources.contains(&source_id) {
            return Err(format!("未找到 source: {}", source_id));
        }
        if !self
            .surfaces
            .list_surfaces(&session_id)
            .iter()
            .any(|surface| surface.surface_id == surface_id)
        {
            return Err(format!("未找到 surface: {}", surface_id));
        }

        self.bindings_by_session
            .entry(session_id)
            .or_default()
            .insert(surface_id, source_id);
        Ok(())
    }

    pub fn snapshot(&self, session_id: &SessionId) -> SessionLifecycleSnapshot {
        let surfaces = self.surfaces.list_surfaces(session_id);
        let available_source_ids = self
            .available_sources_by_session
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        let surface_source_bindings = self
            .bindings_by_session
            .get(session_id)
            .map(|bindings| {
                bindings
                    .iter()
                    .map(|(surface_id, source_id)| SurfaceSourceBinding {
                        surface_id: surface_id.clone(),
                        source_id: source_id.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        SessionLifecycleSnapshot {
            session_id: session_id.0.clone(),
            current_surface_id: self.surfaces.current_surface_id(session_id),
            surfaces,
            available_source_ids,
            surface_source_bindings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionLifecycleCoordinator;
    use mrd_proto::SessionId;

    #[test]
    fn snapshot_tracks_surfaces_and_sources() {
        let mut coordinator = SessionLifecycleCoordinator::default();
        let session_id = SessionId("session-a".into());
        let surface = coordinator.create_surface(session_id.clone(), Some("Main".into()));
        coordinator.update_available_sources(
            session_id.clone(),
            vec!["video-track-1".into(), "video-track-2".into()],
        );
        coordinator
            .bind_surface_source(
                session_id.clone(),
                surface.surface_id.clone(),
                "video-track-2".into(),
            )
            .expect("bind surface to source");

        let snapshot = coordinator.snapshot(&session_id);

        assert_eq!(
            snapshot.current_surface_id,
            Some(surface.surface_id.clone())
        );
        assert_eq!(snapshot.available_source_ids.len(), 2);
        assert_eq!(snapshot.surface_source_bindings.len(), 1);
        assert_eq!(
            snapshot.surface_source_bindings[0].source_id,
            "video-track-2"
        );
    }
}
