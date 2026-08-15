use mrd_proto::SessionId;

use crate::{render_host::RenderHost, session_lifecycle::SessionLifecycleCoordinator};

pub fn sync_session_runtime(
    lifecycle: &mut SessionLifecycleCoordinator,
    render_host: &mut RenderHost,
    session_id: &SessionId,
) -> Result<(), String> {
    let render_snapshot = render_host.snapshot(session_id)?;
    lifecycle.update_available_sources(
        session_id.clone(),
        render_snapshot.available_source_ids.clone(),
    );

    let lifecycle_snapshot = lifecycle.snapshot(session_id);
    for binding in lifecycle_snapshot.surface_source_bindings {
        let _ = render_host.bind_surface_source(session_id, &binding.surface_id, binding.source_id);
    }

    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::{Arc, Mutex};

    use mrd_pipeline_core::DecodedFrame;
    use mrd_proto::SessionId;

    use crate::{
        frame_sink::DecodedFrameSink, render_host::RenderHost,
        session_lifecycle::SessionLifecycleCoordinator,
    };

    use super::sync_session_runtime;

    #[test]
    fn syncing_runtime_projects_lifecycle_bindings_into_render_host() {
        let session_id = SessionId("session-runtime".into());
        let sink = Arc::new(Mutex::new(DecodedFrameSink::default()));
        sink.lock()
            .expect("lock decoded frame sink")
            .ingest_frame_for_source(
                session_id.clone(),
                "video-track-2".into(),
                DecodedFrame::from_cpu_rgb24(2, 2, 0, vec![255; 12]),
            );

        let mut render_host = RenderHost::with_frame_sink(sink);
        render_host
            .attach_session(session_id.clone(), "surface-1".into(), 0)
            .expect("attach renderer for session");

        let mut lifecycle = SessionLifecycleCoordinator::default();
        lifecycle.ensure_surface(session_id.clone(), "surface-1".into());
        lifecycle.update_available_sources(session_id.clone(), vec!["video-track-2".into()]);
        lifecycle
            .bind_surface_source(
                session_id.clone(),
                "surface-1".into(),
                "video-track-2".into(),
            )
            .expect("bind source in lifecycle");

        sync_session_runtime(&mut lifecycle, &mut render_host, &session_id)
            .expect("sync session runtime");

        let render_snapshot = render_host
            .snapshot(&session_id)
            .expect("render host snapshot after sync");

        assert_eq!(
            render_snapshot.available_source_ids,
            vec!["video-track-2".to_string()]
        );
        assert_eq!(render_snapshot.surface_source_bindings.len(), 1);
        assert_eq!(
            render_snapshot.surface_source_bindings[0].surface_id,
            "surface-1".to_string()
        );
        assert_eq!(
            render_snapshot.surface_source_bindings[0].source_id,
            "video-track-2".to_string()
        );
    }
}
