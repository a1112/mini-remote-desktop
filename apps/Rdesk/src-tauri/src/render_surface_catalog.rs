use std::collections::HashMap;

use mrd_proto::SessionId;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RenderSurfaceDescriptor {
    pub surface_id: String,
    pub name: String,
    pub role: String,
}

#[derive(Default)]
pub struct RenderSurfaceCatalog {
    next_ids: HashMap<SessionId, u32>,
    surfaces_by_session: HashMap<SessionId, Vec<RenderSurfaceDescriptor>>,
    current_surface_by_session: HashMap<SessionId, String>,
}

impl RenderSurfaceCatalog {
    pub fn create_surface(
        &mut self,
        session_id: SessionId,
        name: Option<String>,
    ) -> RenderSurfaceDescriptor {
        let next_id = self.next_ids.entry(session_id.clone()).or_insert(1);
        let surface = RenderSurfaceDescriptor {
            surface_id: format!("surface-{}", *next_id),
            name: name.unwrap_or_else(|| format!("Surface {}", *next_id)),
            role: "controller".to_string(),
        };
        *next_id += 1;

        let surfaces = self.surfaces_by_session.entry(session_id.clone()).or_default();
        surfaces.push(surface.clone());
        self.current_surface_by_session
            .insert(session_id, surface.surface_id.clone());
        surface
    }

    pub fn ensure_surface(
        &mut self,
        session_id: SessionId,
        surface_id: String,
    ) -> RenderSurfaceDescriptor {
        let surfaces = self.surfaces_by_session.entry(session_id.clone()).or_default();
        if let Some(surface) = surfaces
            .iter()
            .find(|surface| surface.surface_id == surface_id)
            .cloned()
        {
            self.current_surface_by_session
                .entry(session_id)
                .or_insert_with(|| surface.surface_id.clone());
            return surface;
        }

        let surface = RenderSurfaceDescriptor {
            name: format!("Surface {}", surface_id),
            surface_id: surface_id.clone(),
            role: "controller".to_string(),
        };
        surfaces.push(surface.clone());
        self.current_surface_by_session.insert(session_id, surface_id);
        surface
    }

    pub fn list_surfaces(&self, session_id: &SessionId) -> Vec<RenderSurfaceDescriptor> {
        self.surfaces_by_session
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn select_current_surface(
        &mut self,
        session_id: SessionId,
        surface_id: String,
    ) -> Result<(), String> {
        let surfaces = self
            .surfaces_by_session
            .get(&session_id)
            .ok_or_else(|| format!("未找到会话 surface: {}", session_id.0))?;
        if !surfaces.iter().any(|surface| surface.surface_id == surface_id) {
            return Err(format!("未找到 surface: {}", surface_id));
        }
        self.current_surface_by_session.insert(session_id, surface_id);
        Ok(())
    }

    pub fn current_surface_id(&self, session_id: &SessionId) -> Option<String> {
        self.current_surface_by_session.get(session_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::RenderSurfaceCatalog;
    use mrd_proto::SessionId;

    #[test]
    fn creating_surface_marks_it_current() {
        let mut catalog = RenderSurfaceCatalog::default();
        let surface = catalog.create_surface(SessionId("session-a".into()), Some("Screen A".into()));

        assert_eq!(surface.surface_id, "surface-1");
        assert_eq!(surface.name, "Screen A");
        assert_eq!(
            catalog.current_surface_id(&SessionId("session-a".into())),
            Some("surface-1".into())
        );
    }

    #[test]
    fn selecting_existing_surface_switches_current_pointer() {
        let mut catalog = RenderSurfaceCatalog::default();
        catalog.create_surface(SessionId("session-a".into()), None);
        let second = catalog.create_surface(SessionId("session-a".into()), None);

        catalog
            .select_current_surface(SessionId("session-a".into()), second.surface_id.clone())
            .expect("select second surface");

        assert_eq!(
            catalog.current_surface_id(&SessionId("session-a".into())),
            Some(second.surface_id)
        );
    }
}
