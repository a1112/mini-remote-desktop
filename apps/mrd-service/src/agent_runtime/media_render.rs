//! Exact logical-session routing for service-to-agent encoded render media.

use mrd_agent_ipc::{MediaCodec, RenderAccessUnit};
use mrd_proto::SessionId;
use std::collections::HashMap;
use thiserror::Error;

/// Fail-closed render-route registry errors.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum AgentRenderRouteError {
    /// Persisted binding does not authorize render routing.
    #[error("agent binding does not authorize rendering")]
    InvalidBinding,
    /// One logical session may own only one live route.
    #[error("agent render route already exists for the session")]
    DuplicateSession,
    /// The bounded registry cannot admit another session.
    #[error("agent render route capacity is exhausted")]
    CapacityExceeded,
    /// Render resource ids may not use the zero sentinel.
    #[error("agent render resource id is invalid")]
    InvalidResource,
    /// No installed route owns the logical session.
    #[error("agent render route is unavailable for the session")]
    MissingSession,
    /// Access-unit sequence must strictly increase within the route.
    #[error("agent render access-unit sequence is not monotonic")]
    NonMonotonicSequence,
    /// The resulting bounded IPC unit is invalid.
    #[error("agent render access unit is invalid")]
    InvalidUnit,
}

/// Receiver decision after attempting the agent render boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRenderDispatch {
    /// The encoded unit was queued to the exact agent connection.
    Delivered,
    /// No agent render route is installed; the local migration fallback may run.
    Unavailable,
    /// A route exists but validation or delivery failed; do not bypass it locally.
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentRenderRoute<B> {
    binding: B,
    resource_id: [u8; 16],
    last_sequence: u64,
}

/// Exact binding and validated unit prepared under one registry mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAgentRender<B> {
    binding: B,
    unit: RenderAccessUnit,
}

impl<B> PreparedAgentRender<B> {
    /// Persisted exact agent binding selected for the unit.
    pub fn binding(&self) -> &B {
        &self.binding
    }

    /// Validated bounded IPC unit.
    pub fn unit(&self) -> &RenderAccessUnit {
        &self.unit
    }

    /// Consume the prepared route into its delivery parts.
    pub fn into_parts(self) -> (B, RenderAccessUnit) {
        (self.binding, self.unit)
    }
}

/// Bounded session-to-agent render route registry.
#[derive(Debug)]
pub struct AgentRenderRouteRegistry<B> {
    capacity: usize,
    routes: HashMap<SessionId, AgentRenderRoute<B>>,
}

impl<B> AgentRenderRouteRegistry<B>
where
    B: Clone,
{
    /// Create a registry with a nonzero session limit.
    pub fn new(capacity: usize) -> Option<Self> {
        (capacity > 0).then_some(Self {
            capacity,
            routes: HashMap::with_capacity(capacity),
        })
    }

    /// Install one exact binding/resource pair without implicit replacement.
    pub fn install(
        &mut self,
        session_id: SessionId,
        binding: B,
        resource_id: [u8; 16],
    ) -> Result<(), AgentRenderRouteError> {
        if resource_id == [0; 16] {
            return Err(AgentRenderRouteError::InvalidResource);
        }
        if self.routes.contains_key(&session_id) {
            return Err(AgentRenderRouteError::DuplicateSession);
        }
        if self.routes.len() >= self.capacity {
            return Err(AgentRenderRouteError::CapacityExceeded);
        }
        self.routes.insert(
            session_id,
            AgentRenderRoute {
                binding,
                resource_id,
                last_sequence: 0,
            },
        );
        Ok(())
    }

    /// Prepare one validated IPC unit and advance the route sequence atomically.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        session_id: &SessionId,
        sequence: u64,
        timestamp_us: u64,
        codec: MediaCodec,
        is_keyframe: bool,
        payload: Vec<u8>,
    ) -> Result<PreparedAgentRender<B>, AgentRenderRouteError> {
        let route = self
            .routes
            .get_mut(session_id)
            .ok_or(AgentRenderRouteError::MissingSession)?;
        if sequence <= route.last_sequence {
            return Err(AgentRenderRouteError::NonMonotonicSequence);
        }
        let unit = RenderAccessUnit {
            resource_id: route.resource_id,
            session_id: session_id.0.clone(),
            sequence,
            timestamp_us,
            codec,
            is_keyframe,
            payload,
        };
        if !unit.is_valid() {
            return Err(AgentRenderRouteError::InvalidUnit);
        }
        route.last_sequence = sequence;
        Ok(PreparedAgentRender {
            binding: route.binding.clone(),
            unit,
        })
    }

    /// Explicitly revoke one route and return its exact binding.
    pub fn remove(&mut self, session_id: &SessionId) -> Option<B> {
        self.routes.remove(session_id).map(|route| route.binding)
    }
}
