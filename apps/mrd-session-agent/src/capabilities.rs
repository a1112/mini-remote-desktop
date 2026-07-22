//! Capabilities coupled to the concrete command backend.

use mrd_agent_ipc::{AgentCapability, AgentCommand};
use std::collections::BTreeSet;

/// Capabilities backed by handlers in this process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCapabilities {
    implemented: BTreeSet<AgentCapability>,
}

impl AgentCapabilities {
    /// The Task 22 shell has no product handlers and therefore no capabilities.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Construct capabilities for a concrete backend implementation.
    ///
    /// Callers that inject a backend must derive this set from the same backend,
    /// never from remote claims or a product configuration flag.
    pub fn from_implemented(capabilities: impl IntoIterator<Item = AgentCapability>) -> Self {
        Self {
            implemented: capabilities.into_iter().collect(),
        }
    }

    /// Return the ordered wire capability set.
    pub fn as_set(&self) -> &BTreeSet<AgentCapability> {
        &self.implemented
    }

    /// Whether the backend implements no product behavior.
    pub fn is_empty(&self) -> bool {
        self.implemented.is_empty()
    }

    /// Whether this backend may receive the command after authorization.
    ///
    /// A backend must keep advertising an implemented cleanup handler for as
    /// long as resources of that kind can exist; transient device availability
    /// must not remove the capability needed to stop those resources.
    pub fn supports_command(&self, command: &AgentCommand) -> bool {
        self.implemented.contains(&command.required_capability())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_shell_advertises_no_product_capabilities() {
        let capabilities = AgentCapabilities::empty();
        assert!(capabilities.is_empty());
        assert!(!capabilities.supports_command(&AgentCommand::StartCapture {
            resource_id: [1; 16],
            display_id: 0,
        }));
        assert!(!capabilities.supports_command(&AgentCommand::StopCapture {
            resource_id: [1; 16],
        }));
    }
}
