//! Session-local input-resource execution and pressed-state cleanup.

use mrd_agent_ipc::{
    authorize_input_resource, validate_input_event, AgentCommand, AuthorizedCommand,
    AuthorizedInputResource, ExecutionContext, InputAckOutcome, InputButton, InputEventEnvelope,
    InputEventPayload, InputFailure, InputKey, InputRejection, ValidatedInputEvent,
};
use mrd_input::{InputError, InputEvent, InputInjector, TrackedInputInjector};
use mrd_proto::SessionId;
use std::collections::{HashMap, HashSet};

const REPLAY_CACHE_CAPACITY: usize = 4_096;

#[derive(Debug, Clone)]
struct InputResourceState {
    resource: AuthorizedInputResource,
    pressed_buttons: HashSet<InputButton>,
    pressed_keys: HashSet<InputKey>,
    last_sequence: u64,
    replay: HashMap<u64, ([u8; 32], InputAckOutcome)>,
}

/// Input executor bound to one desktop agent process.
///
/// The manager keeps per-resource pressed state while sharing physical key and
/// button transitions between resources. Stopping one resource therefore never
/// releases a key still held by another resource.
#[derive(Debug)]
pub struct InputResourceManager<I> {
    injector: TrackedInputInjector<I>,
    resources: HashMap<[u8; 16], InputResourceState>,
    button_holders: HashMap<InputButton, usize>,
    key_holders: HashMap<InputKey, usize>,
}

/// Runtime-facing input backend boundary.
pub trait InputBackend: Send {
    /// Whether this exact platform backend is currently available.
    fn is_available(&self) -> bool;
    /// Establish an input resource from a validated StartInput command.
    fn start(&mut self, command: mrd_agent_ipc::AuthorizedCommand) -> Result<(), InputRejection>;
    /// Process one event using the current trusted execution context.
    fn handle(
        &mut self,
        envelope: &InputEventEnvelope,
        context: &ExecutionContext,
    ) -> InputAckOutcome;
    /// Stop one resource and release its pressed state.
    fn stop(&mut self, resource_id: &[u8; 16]) -> InputAckOutcome;
    /// Release pressed state and remove resources owned by one product session.
    fn release_session(&mut self, session_id: &SessionId) -> Result<(), InputError>;
    /// Release all pressed state and clear resources.
    fn release_all(&mut self) -> Result<(), InputError>;
}

impl<I: InputInjector> InputResourceManager<I> {
    /// Construct an empty manager around a platform injector.
    pub fn new(injector: I) -> Self {
        Self {
            injector: TrackedInputInjector::new(injector),
            resources: HashMap::new(),
            button_holders: HashMap::new(),
            key_holders: HashMap::new(),
        }
    }

    /// Whether the underlying platform injector is available.
    pub fn is_available(&self) -> bool {
        self.injector.is_available()
    }

    /// Establish one input resource from an already-validated StartInput command.
    pub fn start(&mut self, command: AuthorizedCommand) -> Result<(), InputRejection> {
        if !matches!(command.command(), AgentCommand::StartInput { .. }) {
            return Err(InputRejection::Grant);
        }
        if !self.is_available() {
            return Err(InputRejection::Unsupported);
        }
        let resource = authorize_input_resource(command).map_err(|_| InputRejection::Grant)?;
        let id = *resource.resource_id();
        if self.resources.contains_key(&id)
            || self
                .resources
                .values()
                .any(|active| active.resource.start_grant_id() == resource.start_grant_id())
        {
            return Err(InputRejection::Replay);
        }
        self.resources.insert(
            id,
            InputResourceState {
                resource,
                pressed_buttons: HashSet::new(),
                pressed_keys: HashSet::new(),
                last_sequence: 0,
                replay: HashMap::new(),
            },
        );
        Ok(())
    }

    /// Process one resource-bound event and return a payload-free acknowledgment.
    pub fn handle(
        &mut self,
        envelope: &InputEventEnvelope,
        context: &ExecutionContext,
    ) -> InputAckOutcome {
        let id = envelope.resource_id;
        let Some(resource) = self.resources.get(&id) else {
            return InputAckOutcome::Rejected {
                reason: InputRejection::Grant,
            };
        };
        let commitment = match envelope.commitment() {
            Ok(value) => value,
            Err(reason) => return InputAckOutcome::Rejected { reason },
        };
        if let Some((cached_commitment, cached)) = resource.replay.get(&envelope.sequence) {
            return if cached_commitment == &commitment {
                *cached
            } else {
                InputAckOutcome::Rejected {
                    reason: InputRejection::Replay,
                }
            };
        }
        let validated = match validate_input_event(envelope, &resource.resource, context) {
            Ok(value) => value,
            Err(reason) => return InputAckOutcome::Rejected { reason },
        };
        if envelope.sequence <= resource.last_sequence {
            return InputAckOutcome::Rejected {
                reason: InputRejection::Replay,
            };
        }

        let outcome = self.inject_validated(id, validated);
        let resource = self
            .resources
            .get_mut(&id)
            .expect("resource remains installed");
        resource.last_sequence = envelope.sequence;
        resource
            .replay
            .insert(envelope.sequence, (commitment, outcome));
        if resource.replay.len() > REPLAY_CACHE_CAPACITY {
            if let Some(oldest) = resource.replay.keys().copied().min() {
                resource.replay.remove(&oldest);
            }
        }
        outcome
    }

    /// Stop one validated resource and release its pressed state.
    pub fn stop(&mut self, resource_id: &[u8; 16]) -> InputAckOutcome {
        let Some(resource) = self.resources.get(resource_id).cloned() else {
            return InputAckOutcome::Rejected {
                reason: InputRejection::Grant,
            };
        };
        match self.release_state(&resource) {
            Ok(()) => {
                self.resources.remove(resource_id);
                InputAckOutcome::Applied
            }
            Err(error) => map_input_error(error),
        }
    }

    /// Release pressed state and remove every resource owned by one session.
    pub fn release_session(&mut self, session_id: &SessionId) -> Result<(), InputError> {
        let resources = self
            .resources
            .values()
            .filter(|state| state.resource.session_id() == session_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut first_error = None;
        for resource in &resources {
            match self.release_state(resource) {
                Ok(()) => {
                    self.resources.remove(resource.resource.resource_id());
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Release every pressed input and remove all input resources.
    pub fn release_all(&mut self) -> Result<(), InputError> {
        let resources = self.resources.values().cloned().collect::<Vec<_>>();
        let mut first_error = None;
        for resource in &resources {
            match self.release_state(resource) {
                Ok(()) => {
                    self.resources.remove(resource.resource.resource_id());
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        self.button_holders.clear();
        self.key_holders.clear();
        Ok(())
    }

    fn inject_validated(
        &mut self,
        resource_id: [u8; 16],
        validated: ValidatedInputEvent,
    ) -> InputAckOutcome {
        let event = validated.event();
        let result = match *event {
            InputEventPayload::MouseMove { x, y } => {
                self.injector.inject(&InputEvent::MouseMove { x, y })
            }
            InputEventPayload::MouseWheel { delta } => {
                self.injector.inject(&InputEvent::MouseWheel { delta })
            }
            InputEventPayload::MouseHorizontalWheel { delta } => self
                .injector
                .inject(&InputEvent::MouseHorizontalWheel { delta }),
            InputEventPayload::MouseButton { button, pressed } => {
                self.transition_button(resource_id, button, pressed)
            }
            InputEventPayload::Key { key, pressed } => {
                self.transition_key(resource_id, key, pressed)
            }
            InputEventPayload::ReleaseAll => self
                .resources
                .get(&resource_id)
                .cloned()
                .map(|resource| self.release_state(&resource))
                .unwrap_or(Ok(())),
        };
        match result {
            Ok(()) => InputAckOutcome::Applied,
            Err(error) => map_input_error(error),
        }
    }

    fn transition_button(
        &mut self,
        resource_id: [u8; 16],
        button: InputButton,
        pressed: bool,
    ) -> Result<(), InputError> {
        let state = self
            .resources
            .get_mut(&resource_id)
            .expect("resource exists");
        let held = state.pressed_buttons.contains(&button);
        if held == pressed {
            return Ok(());
        }
        let holders = self.button_holders.get(&button).copied().unwrap_or(0);
        if (pressed && holders == 0) || (!pressed && holders == 1) {
            self.injector.inject(&InputEvent::MouseButton {
                button: to_input_button(button),
                pressed,
            })?;
        }
        if pressed {
            state.pressed_buttons.insert(button);
            self.button_holders.insert(button, holders + 1);
        } else {
            state.pressed_buttons.remove(&button);
            if holders <= 1 {
                self.button_holders.remove(&button);
            } else {
                self.button_holders.insert(button, holders - 1);
            }
        }
        Ok(())
    }

    fn transition_key(
        &mut self,
        resource_id: [u8; 16],
        key: InputKey,
        pressed: bool,
    ) -> Result<(), InputError> {
        let state = self
            .resources
            .get_mut(&resource_id)
            .expect("resource exists");
        let held = state.pressed_keys.contains(&key);
        if held == pressed {
            return Ok(());
        }
        let holders = self.key_holders.get(&key).copied().unwrap_or(0);
        if (pressed && holders == 0) || (!pressed && holders == 1) {
            self.injector.inject(&InputEvent::Key {
                key: to_input_key(key),
                pressed,
            })?;
        }
        if pressed {
            state.pressed_keys.insert(key);
            self.key_holders.insert(key, holders + 1);
        } else {
            state.pressed_keys.remove(&key);
            if holders <= 1 {
                self.key_holders.remove(&key);
            } else {
                self.key_holders.insert(key, holders - 1);
            }
        }
        Ok(())
    }

    fn release_state(&mut self, state: &InputResourceState) -> Result<(), InputError> {
        let mut first_error = None;
        for button in state.pressed_buttons.iter().copied().collect::<Vec<_>>() {
            if let Err(error) = self.transition_button(*state.resource.resource_id(), button, false)
            {
                first_error.get_or_insert(error);
            }
        }
        for key in state.pressed_keys.iter().copied().collect::<Vec<_>>() {
            if let Err(error) = self.transition_key(*state.resource.resource_id(), key, false) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl<I: InputInjector + Send> InputBackend for InputResourceManager<I> {
    fn is_available(&self) -> bool {
        InputResourceManager::is_available(self)
    }

    fn start(&mut self, command: mrd_agent_ipc::AuthorizedCommand) -> Result<(), InputRejection> {
        Self::start(self, command)
    }

    fn handle(
        &mut self,
        envelope: &InputEventEnvelope,
        context: &ExecutionContext,
    ) -> InputAckOutcome {
        Self::handle(self, envelope, context)
    }

    fn stop(&mut self, resource_id: &[u8; 16]) -> InputAckOutcome {
        Self::stop(self, resource_id)
    }

    fn release_session(&mut self, session_id: &SessionId) -> Result<(), InputError> {
        Self::release_session(self, session_id)
    }

    fn release_all(&mut self) -> Result<(), InputError> {
        Self::release_all(self)
    }
}

fn to_input_button(button: InputButton) -> mrd_input::InputButton {
    match button {
        InputButton::Left => mrd_input::InputButton::Left,
        InputButton::Right => mrd_input::InputButton::Right,
        InputButton::Middle => mrd_input::InputButton::Middle,
        InputButton::X1 => mrd_input::InputButton::Other(1),
        InputButton::X2 => mrd_input::InputButton::Other(2),
    }
}

fn to_input_key(key: InputKey) -> mrd_input::InputKey {
    match key {
        InputKey::VirtualKey { code } => mrd_input::InputKey::VirtualKey(code),
    }
}

fn map_input_error(error: InputError) -> InputAckOutcome {
    match error {
        InputError::Unavailable(_) => InputAckOutcome::Rejected {
            reason: InputRejection::Unsupported,
        },
        InputError::InvalidEvent(_) => InputAckOutcome::Rejected {
            reason: InputRejection::InvalidEvent,
        },
        InputError::UipiDenied => InputAckOutcome::Failed {
            reason: InputFailure::Uipi,
        },
        InputError::Platform(_) => InputAckOutcome::Failed {
            reason: InputFailure::Platform,
        },
    }
}
