use mrd_input::{InputButton, InputError, InputEvent, InputInjector, InputKey};
use mrd_ipc::{
    ControlChannelLaneSnapshot, ControlChannelReliability, ControlChannelSnapshot,
    ControlInputButton, ControlInputEvent, ControlInputKey, ControlInputLane,
};
use mrd_proto::SessionId;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlInputResult {
    pub lane: ControlInputLane,
    pub event_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg(any(target_os = "macos", test))]
pub struct ControlInputTargetGeometry {
    pub frame_width: u32,
    pub frame_height: u32,
    pub source_width: f64,
    pub source_height: f64,
    pub origin_x: f64,
    pub origin_y: f64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg(any(target_os = "macos", test))]
struct MacosInputTargetBinding {
    source: mrd_ipc::CaptureSource,
    profile: mrd_ipc::MediaProfileNegotiation,
}

#[derive(Debug, Clone, Default)]
struct ControlLaneCounters {
    accepted_messages: u64,
    injected_messages: u64,
    failed_messages: u64,
    dropped_messages: u64,
    coalesced_messages: u64,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlInputScope {
    Pointer,
    Keyboard,
}

#[derive(Debug, Default)]
struct SessionPressedInput {
    buttons: HashSet<InputButton>,
    keys: HashSet<InputKey>,
}

pub struct ControlInputRegistry {
    injector: Box<dyn InputInjector>,
    reliable: ControlLaneCounters,
    realtime: ControlLaneCounters,
    last_realtime_mouse_move_by_session: HashMap<SessionId, (i32, i32)>,
    pressed_by_session: HashMap<SessionId, SessionPressedInput>,
    migration_frozen_sessions: HashSet<SessionId>,
    button_holder_counts: HashMap<InputButton, usize>,
    key_holder_counts: HashMap<InputKey, usize>,
}

impl ControlInputRegistry {
    pub fn default_for_platform() -> Self {
        #[cfg(windows)]
        let injector: Box<dyn InputInjector> =
            Box::new(mrd_input::windows::WindowsSendInputInjector::new());

        #[cfg(target_os = "macos")]
        let injector: Box<dyn InputInjector> =
            Box::new(mrd_input::macos::MacosInputInjector::new());

        #[cfg(not(any(windows, target_os = "macos")))]
        let injector: Box<dyn InputInjector> = Box::new(mrd_input::UnsupportedInputInjector::new(
            "input injection is not implemented for this platform",
        ));

        Self::with_injector(injector)
    }

    pub fn with_injector<I>(injector: I) -> Self
    where
        I: InputInjector + 'static,
    {
        Self {
            injector: Box::new(injector),
            reliable: ControlLaneCounters::default(),
            realtime: ControlLaneCounters::default(),
            last_realtime_mouse_move_by_session: HashMap::new(),
            pressed_by_session: HashMap::new(),
            migration_frozen_sessions: HashSet::new(),
            button_holder_counts: HashMap::new(),
            key_holder_counts: HashMap::new(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.injector.is_available()
    }

    #[cfg(test)]
    pub fn handle_event(
        &mut self,
        event: &ControlInputEvent,
    ) -> Result<ControlInputResult, InputError> {
        self.handle_event_inner(None, event, None)
    }

    pub fn handle_session_event(
        &mut self,
        session_id: &SessionId,
        event: &ControlInputEvent,
    ) -> Result<ControlInputResult, InputError> {
        self.handle_event_inner(Some(session_id), event, None)
    }

    pub(crate) fn handle_authenticated_session_event(
        &mut self,
        session_id: &SessionId,
        scope: ControlInputScope,
        event: &ControlInputEvent,
    ) -> Result<ControlInputResult, InputError> {
        self.handle_event_inner(Some(session_id), event, Some(scope))
    }

    #[cfg(any(windows, test))]
    fn begin_agent_event(
        &mut self,
        session_id: &SessionId,
        event: &ControlInputEvent,
    ) -> Result<bool, InputError> {
        let lane = input_lane(event);
        counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane).accepted_messages += 1;
        if self.migration_frozen_sessions.contains(session_id)
            && !matches!(event, ControlInputEvent::ReleaseAll)
        {
            counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane).dropped_messages +=
                1;
            return Err(InputError::InvalidEvent(
                "control input is frozen during relay migration".into(),
            ));
        }
        if self.should_coalesce_realtime_event(Some(session_id), event) {
            counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane)
                .coalesced_messages += 1;
            return Ok(false);
        }
        Ok(true)
    }

    #[cfg(any(windows, test))]
    fn finish_agent_event(
        &mut self,
        session_id: &SessionId,
        scope: ControlInputScope,
        event: &ControlInputEvent,
        result: Result<u32, InputError>,
    ) -> Result<ControlInputResult, InputError> {
        let lane = input_lane(event);
        match result {
            Ok(event_count) => {
                self.record_successful_realtime_event(Some(session_id), event, Some(scope));
                let counter = counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane);
                counter.injected_messages += u64::from(event_count);
                counter.last_error = None;
                Ok(ControlInputResult { lane, event_count })
            }
            Err(error) => {
                let counter = counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane);
                counter.failed_messages += 1;
                counter.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn handle_event_inner(
        &mut self,
        session_id: Option<&SessionId>,
        event: &ControlInputEvent,
        release_scope: Option<ControlInputScope>,
    ) -> Result<ControlInputResult, InputError> {
        let lane = input_lane(event);
        counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane).accepted_messages += 1;

        if session_id.is_some_and(|session_id| {
            self.migration_frozen_sessions.contains(session_id)
                && !matches!(event, ControlInputEvent::ReleaseAll)
        }) {
            counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane).dropped_messages +=
                1;
            return Err(InputError::InvalidEvent(
                "control input is frozen during relay migration".into(),
            ));
        }

        if self.should_coalesce_realtime_event(session_id, event) {
            counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane)
                .coalesced_messages += 1;
            return Ok(ControlInputResult {
                lane,
                event_count: 0,
            });
        }

        let result: Result<u32, InputError> = match (session_id, event) {
            (Some(session_id), ControlInputEvent::ReleaseAll) => match release_scope {
                Some(scope) => self.release_session_scope(session_id, scope),
                None => self.release_session_all(session_id),
            },
            (None, ControlInputEvent::ReleaseAll) => Ok(0),
            (Some(session_id), event) => input_event_from_ipc(event)
                .and_then(|input| self.inject_session_input(session_id, input)),
            (None, event) => input_event_from_ipc(event)
                .and_then(|input| self.injector.inject(&input))
                .map(|()| 1),
        };

        match result {
            Ok(event_count) => {
                self.record_successful_realtime_event(session_id, event, release_scope);
                let counter = counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane);
                counter.injected_messages += u64::from(event_count);
                counter.last_error = None;
                Ok(ControlInputResult { lane, event_count })
            }
            Err(error) => {
                let counter = counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane);
                counter.failed_messages += 1;
                counter.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn should_coalesce_realtime_event(
        &self,
        session_id: Option<&SessionId>,
        event: &ControlInputEvent,
    ) -> bool {
        let Some(session_id) = session_id else {
            return false;
        };
        match *event {
            ControlInputEvent::MouseMove { x, y } => self
                .last_realtime_mouse_move_by_session
                .get(session_id)
                .is_some_and(|last| *last == (x, y)),
            _ => false,
        }
    }

    fn record_successful_realtime_event(
        &mut self,
        session_id: Option<&SessionId>,
        event: &ControlInputEvent,
        release_scope: Option<ControlInputScope>,
    ) {
        let Some(session_id) = session_id else {
            return;
        };
        match *event {
            ControlInputEvent::MouseMove { x, y } => {
                self.last_realtime_mouse_move_by_session
                    .insert(session_id.clone(), (x, y));
            }
            ControlInputEvent::ReleaseAll
                if release_scope.is_none() || release_scope == Some(ControlInputScope::Pointer) =>
            {
                self.last_realtime_mouse_move_by_session.remove(session_id);
            }
            _ => {}
        }
    }

    fn inject_session_input(
        &mut self,
        session_id: &SessionId,
        event: InputEvent,
    ) -> Result<u32, InputError> {
        match event {
            InputEvent::MouseButton { button, pressed } => {
                self.transition_session_button(session_id, button, pressed)
            }
            InputEvent::Key { key, pressed } => {
                self.transition_session_key(session_id, key, pressed)
            }
            event => self.injector.inject(&event).map(|()| 1),
        }
    }

    fn transition_session_button(
        &mut self,
        session_id: &SessionId,
        button: InputButton,
        pressed: bool,
    ) -> Result<u32, InputError> {
        let held_by_session = self
            .pressed_by_session
            .get(session_id)
            .is_some_and(|state| state.buttons.contains(&button));
        if pressed == held_by_session {
            return Ok(0);
        }
        let holders = self.button_holder_counts.get(&button).copied().unwrap_or(0);
        let physical_transition = (pressed && holders == 0) || (!pressed && holders == 1);
        if physical_transition {
            self.injector
                .inject(&InputEvent::MouseButton { button, pressed })?;
        }
        if pressed {
            self.pressed_by_session
                .entry(session_id.clone())
                .or_default()
                .buttons
                .insert(button);
            self.button_holder_counts.insert(button, holders + 1);
        } else {
            if let Some(state) = self.pressed_by_session.get_mut(session_id) {
                state.buttons.remove(&button);
            }
            if holders <= 1 {
                self.button_holder_counts.remove(&button);
            } else {
                self.button_holder_counts.insert(button, holders - 1);
            }
            self.remove_empty_session_state(session_id);
        }
        Ok(u32::from(physical_transition))
    }

    fn transition_session_key(
        &mut self,
        session_id: &SessionId,
        key: InputKey,
        pressed: bool,
    ) -> Result<u32, InputError> {
        let held_by_session = self
            .pressed_by_session
            .get(session_id)
            .is_some_and(|state| state.keys.contains(&key));
        if pressed == held_by_session {
            return Ok(0);
        }
        let holders = self.key_holder_counts.get(&key).copied().unwrap_or(0);
        let physical_transition = (pressed && holders == 0) || (!pressed && holders == 1);
        if physical_transition {
            self.injector.inject(&InputEvent::Key { key, pressed })?;
        }
        if pressed {
            self.pressed_by_session
                .entry(session_id.clone())
                .or_default()
                .keys
                .insert(key);
            self.key_holder_counts.insert(key, holders + 1);
        } else {
            if let Some(state) = self.pressed_by_session.get_mut(session_id) {
                state.keys.remove(&key);
            }
            if holders <= 1 {
                self.key_holder_counts.remove(&key);
            } else {
                self.key_holder_counts.insert(key, holders - 1);
            }
            self.remove_empty_session_state(session_id);
        }
        Ok(u32::from(physical_transition))
    }

    pub(crate) fn release_session_scope(
        &mut self,
        session_id: &SessionId,
        scope: ControlInputScope,
    ) -> Result<u32, InputError> {
        if scope == ControlInputScope::Pointer {
            self.last_realtime_mouse_move_by_session.remove(session_id);
        }
        let Some(state) = self.pressed_by_session.get(session_id) else {
            self.reset_input_if_idle();
            return Ok(0);
        };
        let buttons = if scope == ControlInputScope::Pointer {
            state.buttons.iter().copied().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let keys = if scope == ControlInputScope::Keyboard {
            state.keys.iter().copied().collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut released = 0_u32;
        for button in buttons {
            released =
                released.saturating_add(self.transition_session_button(session_id, button, false)?);
        }
        for key in keys {
            released =
                released.saturating_add(self.transition_session_key(session_id, key, false)?);
        }
        self.reset_input_if_idle();
        Ok(released)
    }

    fn reset_input_if_idle(&mut self) {
        // Native cursor/click/lock state must not leak into the next session.
        // Keep it intact while any other session still owns a pressed input.
        if self.button_holder_counts.is_empty() && self.key_holder_counts.is_empty() {
            self.injector.reset_idle_state();
        }
    }

    pub(crate) fn release_session_all(
        &mut self,
        session_id: &SessionId,
    ) -> Result<u32, InputError> {
        let pointer = self.release_session_scope(session_id, ControlInputScope::Pointer)?;
        let keyboard = self.release_session_scope(session_id, ControlInputScope::Keyboard)?;
        Ok(pointer.saturating_add(keyboard))
    }

    pub(crate) fn release_all_sessions(&mut self) -> Result<u32, InputError> {
        let mut pending = self
            .pressed_by_session
            .keys()
            .chain(self.last_realtime_mouse_move_by_session.keys())
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        pending.sort_by(|left, right| left.0.cmp(&right.0));
        let mut released = 0_u32;
        let mut last_error = None;
        for _ in 0..3 {
            let mut retry = Vec::new();
            for session_id in pending {
                match self.release_session_all(&session_id) {
                    Ok(count) => released = released.saturating_add(count),
                    Err(error) => {
                        last_error = Some(error);
                        retry.push(session_id);
                    }
                }
            }
            if retry.is_empty() {
                return Ok(released);
            }
            pending = retry;
        }
        match last_error {
            Some(error) => Err(error),
            None => Ok(released),
        }
    }

    /// Release every held input state and atomically reject new input for one migration.
    pub(crate) fn freeze_session_for_migration(
        &mut self,
        session_id: &SessionId,
    ) -> Result<u32, InputError> {
        let released = self.release_session_all(session_id);
        self.migration_frozen_sessions.insert(session_id.clone());
        released
    }

    /// Resume authenticated input after an exact migration generation commits or aborts safely.
    pub(crate) fn thaw_session_after_migration(&mut self, session_id: &SessionId) -> bool {
        self.migration_frozen_sessions.remove(session_id)
    }

    pub(crate) fn session_is_migration_frozen(&self, session_id: &SessionId) -> bool {
        self.migration_frozen_sessions.contains(session_id)
    }

    fn remove_empty_session_state(&mut self, session_id: &SessionId) {
        if self
            .pressed_by_session
            .get(session_id)
            .is_some_and(|state| state.buttons.is_empty() && state.keys.is_empty())
        {
            self.pressed_by_session.remove(session_id);
        }
    }

    pub fn snapshot(&self, session_id: SessionId) -> ControlChannelSnapshot {
        ControlChannelSnapshot {
            session_id,
            reliable: lane_snapshot(
                "ctrl_rel",
                ControlChannelReliability::ReliableOrdered,
                true,
                None,
                &self.reliable,
            ),
            realtime: lane_snapshot(
                "ctrl_rt",
                ControlChannelReliability::UnreliableRealtime,
                false,
                Some(0),
                &self.realtime,
            ),
        }
    }

    #[cfg(test)]
    pub fn injected_message_count(&self) -> u64 {
        self.reliable
            .injected_messages
            .saturating_add(self.realtime.injected_messages)
    }
}

impl Default for ControlInputRegistry {
    fn default() -> Self {
        Self::default_for_platform()
    }
}

/// Network receivers call this only while their authorization gate is held and
/// after validating the signed envelope, lane, scope, replay counter and expiry.
pub(crate) async fn apply_authenticated_input(
    state: &crate::AppState,
    session_id: &SessionId,
    scope: ControlInputScope,
    _remote_sequence: u64,
    remote_expires_at_ms: u64,
    event: &ControlInputEvent,
) -> Result<ControlInputResult, InputError> {
    ensure_input_not_expired(remote_expires_at_ms)?;
    #[cfg(target_os = "macos")]
    let (mapped_event, target_binding) =
        map_authenticated_macos_input(state, session_id, remote_expires_at_ms, event).await?;
    #[cfg(target_os = "macos")]
    let event = &mapped_event;
    #[cfg(target_os = "macos")]
    let _target_guards = if let Some(binding) = target_binding.as_ref() {
        // Keep the existing profiles -> sources lock order. Retain both
        // guards while waiting for input and through synchronous injection.
        let profiles = state.media_profiles.lock().await;
        ensure_input_not_expired(remote_expires_at_ms)?;
        let sources = state.capture_sources.lock().await;
        ensure_input_not_expired(remote_expires_at_ms)?;
        if sources
            .get(session_id)
            .as_ref()
            .map(|selection| &selection.source)
            != Some(&binding.source)
            || profiles.get(session_id).as_ref() != Some(&binding.profile)
        {
            return Err(InputError::InvalidEvent(
                "pointer input target changed before application".into(),
            ));
        }
        Some((profiles, sources))
    } else {
        None
    };
    let registry = state.control_input();
    let mut registry = registry.lock().await;
    // Native geometry queries and registry contention can take longer than the
    // signed event's remaining lifetime. Recheck after every asynchronous wait.
    ensure_input_not_expired(remote_expires_at_ms)?;
    #[cfg(windows)]
    if state.console_capture.is_enabled() {
        // Keep this lock until the Agent acknowledges application. A migration
        // freeze must wait for in-flight input before releasing its resources.
        if !registry.begin_agent_event(session_id, event)? {
            return Ok(ControlInputResult {
                lane: input_lane(event),
                event_count: 0,
            });
        }
        let remote_scope = match scope {
            ControlInputScope::Pointer => mrd_ipc::RemotePermissionScope::InputPointer,
            ControlInputScope::Keyboard => mrd_ipc::RemotePermissionScope::InputKeyboard,
        };
        let result = state
            .console_capture
            .apply_input(
                state,
                session_id,
                remote_scope,
                _remote_sequence,
                remote_expires_at_ms,
                agent_event(event),
            )
            .await
            .map(|ack| u32::from(ack.is_some()))
            .map_err(|_| {
                InputError::Unavailable("approved desktop Agent did not apply input".into())
            });
        return registry.finish_agent_event(session_id, scope, event, result);
    }
    registry.handle_authenticated_session_event(session_id, scope, event)
}

fn ensure_input_not_expired(remote_expires_at_ms: u64) -> Result<(), InputError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| InputError::Unavailable("input clock unavailable".into()))?
        .as_millis();
    if now_ms >= u128::from(remote_expires_at_ms) {
        return Err(InputError::InvalidEvent(
            "signed input event expired before application".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn map_authenticated_macos_input(
    state: &crate::AppState,
    session_id: &SessionId,
    remote_expires_at_ms: u64,
    event: &ControlInputEvent,
) -> Result<(ControlInputEvent, Option<MacosInputTargetBinding>), InputError> {
    if !matches!(event, ControlInputEvent::MouseMove { .. }) {
        return Ok((event.clone(), None));
    }
    // Use the same registry lock order as LAN session commits. The negotiated
    // profile describes decoded video pixels, not capture backing pixels.
    let (source, profile) = {
        let profiles = state.media_profiles.lock().await;
        let sources = state.capture_sources.lock().await;
        let source = sources
            .get(session_id)
            .ok_or_else(|| {
                InputError::InvalidEvent("pointer input has no selected capture source".into())
            })?
            .source;
        let profile = profiles.get(session_id).ok_or_else(|| {
            InputError::InvalidEvent("pointer input has no negotiated media profile".into())
        })?;
        validate_macos_pointer_profile(&source.id, &profile)?;
        (source, profile)
    };
    let query_source = source.clone();
    let bounds = tokio::task::spawn_blocking(move || {
        crate::capture_source::macos_input_bounds(&query_source)
    })
    .await
    .map_err(|_| InputError::Unavailable("macOS capture bounds query failed".into()))?
    .map_err(|error| InputError::InvalidEvent(error.to_string()))?;
    ensure_input_not_expired(remote_expires_at_ms)?;
    // A target switch while Quartz was queried invalidates this event. Never
    // apply geometry from one selected source to another source's video frame.
    {
        let profiles = state.media_profiles.lock().await;
        let sources = state.capture_sources.lock().await;
        if sources
            .get(session_id)
            .as_ref()
            .map(|selection| &selection.source)
            != Some(&source)
            || profiles.get(session_id).as_ref() != Some(&profile)
        {
            return Err(InputError::InvalidEvent(
                "pointer input target changed before application".into(),
            ));
        }
    }
    let mapped_event = map_control_input_event_for_target_geometry(
        event,
        Some(ControlInputTargetGeometry {
            frame_width: profile.selected.width,
            frame_height: profile.selected.height,
            source_width: bounds.width,
            source_height: bounds.height,
            origin_x: bounds.origin_x,
            origin_y: bounds.origin_y,
        }),
    )?;
    Ok((
        mapped_event,
        Some(MacosInputTargetBinding { source, profile }),
    ))
}

#[cfg(any(target_os = "macos", test))]
fn validate_macos_pointer_profile(
    source_id: &str,
    profile: &mrd_ipc::MediaProfileNegotiation,
) -> Result<(), InputError> {
    if profile.selected_source_id.as_deref() != Some(source_id)
        || profile.selected.width == 0
        || profile.selected.height == 0
        || profile
            .selected_width
            .is_some_and(|width| width != profile.selected.width)
        || profile
            .selected_height
            .is_some_and(|height| height != profile.selected.height)
    {
        return Err(InputError::InvalidEvent(
            "pointer input media profile does not match the selected capture source".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn release_authenticated_input(state: &crate::AppState, session_id: &SessionId) {
    let registry = state.control_input();
    let mut registry = registry.lock().await;
    #[cfg(windows)]
    if state.console_capture.is_enabled() {
        if state
            .console_capture
            .stop_input(state, session_id)
            .await
            .is_err()
        {
            tracing::warn!(session_id = %session_id.0, "desktop Agent input cleanup failed");
        }
    }
    let _ = registry.release_session_all(session_id);
}

#[cfg(windows)]
fn agent_event(event: &ControlInputEvent) -> mrd_agent_ipc::InputEventPayload {
    use mrd_agent_ipc::{InputButton as Button, InputEventPayload as Event, InputKey as Key};
    match *event {
        ControlInputEvent::MouseMove { x, y } => Event::MouseMove { x, y },
        ControlInputEvent::MouseWheel { delta } => Event::MouseWheel { delta },
        ControlInputEvent::MouseHorizontalWheel { delta } => Event::MouseHorizontalWheel { delta },
        ControlInputEvent::MouseButton { button, pressed } => Event::MouseButton {
            button: match button {
                ControlInputButton::Left => Button::Left,
                ControlInputButton::Right => Button::Right,
                ControlInputButton::Middle => Button::Middle,
                ControlInputButton::X1 => Button::X1,
                ControlInputButton::X2 => Button::X2,
            },
            pressed,
        },
        ControlInputEvent::Key {
            key: ControlInputKey::VirtualKey { code },
            pressed,
        } => Event::Key {
            key: Key::VirtualKey { code },
            pressed,
        },
        ControlInputEvent::ReleaseAll => Event::ReleaseAll,
    }
}

fn input_lane(event: &ControlInputEvent) -> ControlInputLane {
    match event {
        ControlInputEvent::MouseMove { .. }
        | ControlInputEvent::MouseWheel { .. }
        | ControlInputEvent::MouseHorizontalWheel { .. } => ControlInputLane::Realtime,
        ControlInputEvent::MouseButton { .. } | ControlInputEvent::Key { .. } => {
            ControlInputLane::Reliable
        }
        ControlInputEvent::ReleaseAll => ControlInputLane::Cleanup,
    }
}

fn counter_for_lane_mut<'a>(
    reliable: &'a mut ControlLaneCounters,
    realtime: &'a mut ControlLaneCounters,
    lane: ControlInputLane,
) -> &'a mut ControlLaneCounters {
    match lane {
        ControlInputLane::Reliable | ControlInputLane::Cleanup => reliable,
        ControlInputLane::Realtime => realtime,
    }
}

fn lane_snapshot(
    name: &str,
    reliability: ControlChannelReliability,
    ordered: bool,
    max_retransmits: Option<u16>,
    counters: &ControlLaneCounters,
) -> ControlChannelLaneSnapshot {
    ControlChannelLaneSnapshot {
        name: name.to_string(),
        reliability,
        ordered,
        max_retransmits,
        queued_messages: 0,
        dropped_messages: counters.dropped_messages,
        coalesced_messages: counters.coalesced_messages,
        accepted_messages: counters.accepted_messages,
        injected_messages: counters.injected_messages,
        failed_messages: counters.failed_messages,
        last_error: counters.last_error.clone(),
    }
}

#[cfg(any(target_os = "macos", test))]
pub fn map_control_input_event_for_target_geometry(
    event: &ControlInputEvent,
    geometry: Option<ControlInputTargetGeometry>,
) -> Result<ControlInputEvent, InputError> {
    match *event {
        ControlInputEvent::MouseMove { x, y } => {
            let geometry = geometry.ok_or_else(|| {
                InputError::InvalidEvent("pointer input has no target geometry".into())
            })?;
            let x = scale_target_coordinate(
                x,
                geometry.frame_width,
                geometry.source_width,
                geometry.origin_x,
            )?;
            let y = scale_target_coordinate(
                y,
                geometry.frame_height,
                geometry.source_height,
                geometry.origin_y,
            )?;
            Ok(ControlInputEvent::MouseMove { x, y })
        }
        _ => Ok(event.clone()),
    }
}

#[cfg(any(target_os = "macos", test))]
fn scale_target_coordinate(
    coordinate: i32,
    frame_extent: u32,
    source_extent: f64,
    origin: f64,
) -> Result<i32, InputError> {
    let end = origin + source_extent;
    let first = origin.ceil();
    let last = end.ceil() - 1.0;
    if frame_extent == 0
        || !origin.is_finite()
        || !source_extent.is_finite()
        || source_extent <= 0.0
        || !end.is_finite()
        || first > last
        || first < f64::from(i32::MIN)
        || last > f64::from(i32::MAX)
    {
        return Err(InputError::InvalidEvent(
            "pointer input target geometry is invalid".into(),
        ));
    }
    let coordinate = f64::from(coordinate).clamp(0.0, f64::from(frame_extent));
    let mapped = origin + coordinate * source_extent / f64::from(frame_extent);
    Ok(mapped.round().clamp(first, last) as i32)
}

fn input_event_from_ipc(event: &ControlInputEvent) -> Result<InputEvent, InputError> {
    match *event {
        ControlInputEvent::MouseMove { x, y } => Ok(InputEvent::MouseMove { x, y }),
        ControlInputEvent::MouseWheel { delta } => Ok(InputEvent::MouseWheel { delta }),
        ControlInputEvent::MouseHorizontalWheel { delta } => {
            Ok(InputEvent::MouseHorizontalWheel { delta })
        }
        ControlInputEvent::MouseButton { button, pressed } => Ok(InputEvent::MouseButton {
            button: input_button_from_ipc(button),
            pressed,
        }),
        ControlInputEvent::Key { key, pressed } => Ok(InputEvent::Key {
            key: input_key_from_ipc(key),
            pressed,
        }),
        ControlInputEvent::ReleaseAll => Err(InputError::InvalidEvent(
            "release_all is not a single input event".to_string(),
        )),
    }
}

fn input_button_from_ipc(button: ControlInputButton) -> InputButton {
    match button {
        ControlInputButton::Left => InputButton::Left,
        ControlInputButton::Right => InputButton::Right,
        ControlInputButton::Middle => InputButton::Middle,
        ControlInputButton::X1 => InputButton::Other(1),
        ControlInputButton::X2 => InputButton::Other(2),
    }
}

fn input_key_from_ipc(key: ControlInputKey) -> InputKey {
    match key {
        ControlInputKey::VirtualKey { code } => InputKey::VirtualKey(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    #[test]
    fn native_idle_reset_preserves_other_sessions_pressed_input() {
        struct IdleResetRecorder(Arc<std::sync::atomic::AtomicUsize>);
        impl InputInjector for IdleResetRecorder {
            fn is_available(&self) -> bool {
                true
            }
            fn inject(&mut self, _: &InputEvent) -> Result<(), InputError> {
                Ok(())
            }
            fn reset_idle_state(&mut self) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let resets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut registry = ControlInputRegistry::with_injector(IdleResetRecorder(resets.clone()));
        let first = SessionId("first".into());
        let second = SessionId("second".into());
        let down = ControlInputEvent::Key {
            key: ControlInputKey::VirtualKey { code: 0x41 },
            pressed: true,
        };
        registry.handle_session_event(&first, &down).unwrap();
        registry.handle_session_event(&second, &down).unwrap();
        registry.release_session_all(&first).unwrap();
        assert_eq!(resets.load(std::sync::atomic::Ordering::SeqCst), 0);
        registry.release_session_all(&second).unwrap();
        let idle_resets = resets.load(std::sync::atomic::Ordering::SeqCst);
        assert!(idle_resets > 0);
        // A session with only pointer motion has no pressed-state entry, but
        // still leaves native cursor/click state that cleanup must discard.
        registry
            .handle_session_event(&first, &ControlInputEvent::MouseMove { x: 10, y: 20 })
            .unwrap();
        registry.release_session_all(&first).unwrap();
        assert!(resets.load(std::sync::atomic::Ordering::SeqCst) > idle_resets);
    }

    #[tokio::test]
    async fn input_expiring_while_waiting_for_registry_never_reaches_injector() {
        let state = Arc::new(crate::AppState::new());
        let recorded = Arc::new(StdMutex::new(Vec::new()));
        state
            .replace_control_input_for_test(SharedRecordingInputInjector {
                events: recorded.clone(),
            })
            .await;
        let registry = state.control_input();
        let held = registry.lock().await;
        let deadline = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 30;
        let pending_state = state.clone();
        let pending = tokio::spawn(async move {
            apply_authenticated_input(
                &pending_state,
                &SessionId("queued-expired".into()),
                ControlInputScope::Pointer,
                1,
                deadline,
                &ControlInputEvent::MouseWheel { delta: 120 },
            )
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(held);
        assert!(pending.await.unwrap().is_err());
        assert!(recorded.lock().unwrap().is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn resident_input_without_approved_agent_never_falls_back_to_session_zero() {
        let state = crate::AppState::new();
        state.bind_console_capture_issuer(Arc::new(
            crate::agent_runtime::ExecuteGrantIssuer::from_seed([53; 32]).unwrap(),
        ));
        let recorded = Arc::new(StdMutex::new(Vec::new()));
        state
            .replace_control_input_for_test(SharedRecordingInputInjector {
                events: recorded.clone(),
            })
            .await;
        let result = apply_authenticated_input(
            &state,
            &SessionId("unapproved-agent".into()),
            ControlInputScope::Pointer,
            1,
            u64::MAX,
            &ControlInputEvent::MouseMove { x: 25, y: 50 },
        )
        .await;
        assert!(result.is_err());
        assert!(recorded.lock().unwrap().is_empty());
        assert_eq!(
            state.control_input().lock().await.injected_message_count(),
            0
        );
    }

    #[test]
    fn agent_input_counts_only_applied_ack_and_never_calls_local_injector() {
        let mut registry = ControlInputRegistry::with_injector(
            mrd_input::UnsupportedInputInjector::new("Session 0 must not inject"),
        );
        let session_id = SessionId("agent-applied".into());
        let event = ControlInputEvent::MouseMove { x: 123, y: 456 };
        assert!(registry.begin_agent_event(&session_id, &event).unwrap());
        assert_eq!(registry.injected_message_count(), 0);
        registry
            .finish_agent_event(&session_id, ControlInputScope::Pointer, &event, Ok(1))
            .unwrap();
        assert_eq!(registry.injected_message_count(), 1);
        assert!(!registry.begin_agent_event(&session_id, &event).unwrap());
        assert_eq!(registry.snapshot(session_id).realtime.coalesced_messages, 1);
    }

    #[test]
    fn failed_agent_ack_does_not_coalesce_retry_and_migration_stays_frozen() {
        let mut registry = ControlInputRegistry::with_injector(
            mrd_input::UnsupportedInputInjector::new("Session 0 must not inject"),
        );
        let session_id = SessionId("agent-failed".into());
        let event = ControlInputEvent::MouseMove { x: 12, y: 34 };
        assert!(registry.begin_agent_event(&session_id, &event).unwrap());
        assert!(registry
            .finish_agent_event(
                &session_id,
                ControlInputScope::Pointer,
                &event,
                Err(InputError::Unavailable(
                    "Agent acknowledgement missing".into()
                ))
            )
            .is_err());
        assert!(registry.begin_agent_event(&session_id, &event).unwrap());
        registry.freeze_session_for_migration(&session_id).unwrap();
        assert!(registry.begin_agent_event(&session_id, &event).is_err());
        assert!(registry
            .begin_agent_event(&session_id, &ControlInputEvent::ReleaseAll)
            .unwrap());
        assert_eq!(registry.injected_message_count(), 0);
        assert_eq!(registry.snapshot(session_id).realtime.failed_messages, 1);
    }

    #[derive(Clone)]
    struct SharedRecordingInputInjector {
        events: Arc<StdMutex<Vec<InputEvent>>>,
    }

    impl InputInjector for SharedRecordingInputInjector {
        fn is_available(&self) -> bool {
            true
        }

        fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(*event);
            Ok(())
        }
    }

    struct FailsOnceInputInjector {
        error_message: String,
        should_fail: bool,
    }

    struct FailsKeyReleaseInputInjector;

    impl InputInjector for FailsKeyReleaseInputInjector {
        fn is_available(&self) -> bool {
            true
        }

        fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
            if matches!(event, InputEvent::Key { pressed: false, .. }) {
                return Err(InputError::Platform("key release failed".into()));
            }
            Ok(())
        }
    }

    impl FailsOnceInputInjector {
        fn new(error_message: impl Into<String>) -> Self {
            Self {
                error_message: error_message.into(),
                should_fail: true,
            }
        }
    }

    impl InputInjector for FailsOnceInputInjector {
        fn is_available(&self) -> bool {
            true
        }

        fn inject(&mut self, _event: &InputEvent) -> Result<(), InputError> {
            if self.should_fail {
                self.should_fail = false;
                return Err(InputError::Platform(self.error_message.clone()));
            }
            Ok(())
        }
    }

    #[test]
    fn injected_message_count_combines_global_lane_totals() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let mut registry = ControlInputRegistry::with_injector(SharedRecordingInputInjector {
            events: Arc::clone(&events),
        });

        assert_eq!(registry.injected_message_count(), 0);
        registry
            .handle_event(&ControlInputEvent::Key {
                key: ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            })
            .expect("inject reliable event");
        registry
            .handle_event(&ControlInputEvent::MouseMove { x: 10, y: 20 })
            .expect("inject realtime event");

        assert_eq!(registry.injected_message_count(), 2);
        assert_eq!(events.lock().expect("recorded events").len(), 2);
    }

    #[test]
    fn mouse_move_uses_realtime_lane() {
        assert_eq!(
            input_lane(&ControlInputEvent::MouseMove { x: 1, y: 2 }),
            ControlInputLane::Realtime
        );
    }

    #[test]
    fn mouse_wheel_uses_realtime_lane() {
        assert_eq!(
            input_lane(&ControlInputEvent::MouseWheel { delta: -120 }),
            ControlInputLane::Realtime
        );
    }

    #[test]
    fn horizontal_wheel_uses_realtime_lane() {
        assert_eq!(
            input_lane(&ControlInputEvent::MouseHorizontalWheel { delta: 120 }),
            ControlInputLane::Realtime
        );
    }

    #[test]
    fn duplicate_mouse_moves_are_coalesced_on_realtime_lane() {
        let mut registry =
            ControlInputRegistry::with_injector(mrd_input::RecordingInputInjector::available());
        let session_id = SessionId("control-session".to_string());

        let first = registry
            .handle_session_event(&session_id, &ControlInputEvent::MouseMove { x: 10, y: 20 })
            .expect("first mouse move");
        let duplicate = registry
            .handle_session_event(&session_id, &ControlInputEvent::MouseMove { x: 10, y: 20 })
            .expect("duplicate mouse move");
        let snapshot = registry.snapshot(session_id);

        assert_eq!(first.event_count, 1);
        assert_eq!(duplicate.event_count, 0);
        assert_eq!(snapshot.realtime.accepted_messages, 2);
        assert_eq!(snapshot.realtime.injected_messages, 1);
        assert_eq!(snapshot.realtime.coalesced_messages, 1);
    }

    #[test]
    fn mouse_move_coalescing_is_scoped_to_session() {
        let mut registry =
            ControlInputRegistry::with_injector(mrd_input::RecordingInputInjector::available());
        let first_session = SessionId("first-control-session".to_string());
        let second_session = SessionId("second-control-session".to_string());

        registry
            .handle_session_event(
                &first_session,
                &ControlInputEvent::MouseMove { x: 10, y: 20 },
            )
            .expect("first session mouse move");
        let second = registry
            .handle_session_event(
                &second_session,
                &ControlInputEvent::MouseMove { x: 10, y: 20 },
            )
            .expect("second session same mouse move");
        let snapshot = registry.snapshot(first_session);

        assert_eq!(second.event_count, 1);
        assert_eq!(snapshot.realtime.accepted_messages, 2);
        assert_eq!(snapshot.realtime.injected_messages, 2);
        assert_eq!(snapshot.realtime.coalesced_messages, 0);
    }

    #[test]
    fn terminal_release_clears_session_mouse_move_coalescing_state() {
        let mut registry =
            ControlInputRegistry::with_injector(mrd_input::RecordingInputInjector::available());
        let session_id = SessionId("reused-control-session".to_string());
        let move_event = ControlInputEvent::MouseMove { x: 10, y: 20 };

        registry
            .handle_session_event(&session_id, &move_event)
            .expect("initial mouse move");
        registry
            .release_session_all(&session_id)
            .expect("terminal session release");
        let reused = registry
            .handle_session_event(&session_id, &move_event)
            .expect("same coordinate after session reuse");

        assert_eq!(reused.event_count, 1);
    }

    #[test]
    fn migration_freeze_rejects_new_input_even_when_release_all_fails() {
        let mut registry = ControlInputRegistry::with_injector(FailsKeyReleaseInputInjector);
        let session_id = SessionId("migration-input-safety".into());
        registry
            .handle_session_event(
                &session_id,
                &ControlInputEvent::Key {
                    key: ControlInputKey::VirtualKey { code: 0x41 },
                    pressed: true,
                },
            )
            .expect("key down before migration");

        assert_eq!(
            registry
                .freeze_session_for_migration(&session_id)
                .expect_err("failed ReleaseAll must surface"),
            InputError::Platform("key release failed".into())
        );
        assert!(registry.session_is_migration_frozen(&session_id));
        assert!(matches!(
            registry
                .handle_session_event(&session_id, &ControlInputEvent::MouseMove { x: 1, y: 2 },),
            Err(InputError::InvalidEvent(_))
        ));
    }

    #[test]
    fn key_uses_reliable_lane() {
        assert_eq!(
            input_lane(&ControlInputEvent::Key {
                key: ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            }),
            ControlInputLane::Reliable
        );
    }

    #[test]
    fn successful_input_clears_lane_last_error_after_recovery() {
        let mut registry =
            ControlInputRegistry::with_injector(FailsOnceInputInjector::new("temporary failure"));
        let session_id = SessionId("recovering-control-session".to_string());
        let key_down = ControlInputEvent::Key {
            key: ControlInputKey::VirtualKey { code: 0x41 },
            pressed: true,
        };

        let failed = registry
            .handle_session_event(&session_id, &key_down)
            .expect_err("first injection should fail");
        assert_eq!(
            failed,
            InputError::Platform("temporary failure".to_string())
        );
        let failed_snapshot = registry.snapshot(session_id.clone());
        assert_eq!(failed_snapshot.reliable.failed_messages, 1);
        assert_eq!(
            failed_snapshot.reliable.last_error.as_deref(),
            Some("platform input injection failed: temporary failure")
        );

        let recovered = registry
            .handle_session_event(&session_id, &key_down)
            .expect("second injection should recover");
        let recovered_snapshot = registry.snapshot(session_id);

        assert_eq!(recovered.event_count, 1);
        assert_eq!(recovered_snapshot.reliable.failed_messages, 1);
        assert_eq!(recovered_snapshot.reliable.injected_messages, 1);
        assert_eq!(recovered_snapshot.reliable.last_error, None);
    }

    #[test]
    fn target_geometry_scales_frame_mouse_move_to_capture_source_coordinates() {
        let event = map_control_input_event_for_target_geometry(
            &ControlInputEvent::MouseMove { x: 640, y: 360 },
            Some(ControlInputTargetGeometry {
                frame_width: 1280,
                frame_height: 720,
                source_width: 2560.0,
                source_height: 1440.0,
                origin_x: 0.0,
                origin_y: 0.0,
            }),
        )
        .unwrap();

        assert_eq!(event, ControlInputEvent::MouseMove { x: 1280, y: 720 });
    }

    #[test]
    fn target_geometry_adds_display_origin_and_clamps_to_source_bounds() {
        let event = map_control_input_event_for_target_geometry(
            &ControlInputEvent::MouseMove { x: 1280, y: 720 },
            Some(ControlInputTargetGeometry {
                frame_width: 1280,
                frame_height: 720,
                source_width: 2560.0,
                source_height: 1440.0,
                origin_x: 1920.0,
                origin_y: -120.0,
            }),
        )
        .unwrap();

        assert_eq!(event, ControlInputEvent::MouseMove { x: 4479, y: 1319 });
    }

    #[test]
    fn target_geometry_leaves_non_pointer_events_unchanged() {
        let event = ControlInputEvent::Key {
            key: ControlInputKey::VirtualKey { code: 0x41 },
            pressed: true,
        };

        assert_eq!(
            map_control_input_event_for_target_geometry(
                &event,
                Some(ControlInputTargetGeometry {
                    frame_width: 1280,
                    frame_height: 720,
                    source_width: 2560.0,
                    source_height: 1440.0,
                    origin_x: 1920.0,
                    origin_y: 0.0,
                }),
            )
            .unwrap(),
            event
        );
    }

    #[test]
    fn macos_retina_frame_pixels_map_to_logical_points_with_negative_origin() {
        let geometry = Some(ControlInputTargetGeometry {
            frame_width: 3840,
            frame_height: 2160,
            source_width: 1920.0,
            source_height: 1080.0,
            origin_x: -1920.0,
            origin_y: -120.0,
        });
        assert_eq!(
            map_control_input_event_for_target_geometry(
                &ControlInputEvent::MouseMove { x: 1920, y: 1080 },
                geometry,
            )
            .unwrap(),
            ControlInputEvent::MouseMove { x: -960, y: 420 },
        );
        assert_eq!(
            map_control_input_event_for_target_geometry(
                &ControlInputEvent::MouseMove { x: 3839, y: 2159 },
                geometry,
            )
            .unwrap(),
            ControlInputEvent::MouseMove { x: -1, y: 959 },
        );
    }

    #[test]
    fn macos_scaled_window_frame_maps_to_global_bounds_without_y_flip() {
        let geometry = Some(ControlInputTargetGeometry {
            frame_width: 640,
            frame_height: 360,
            source_width: 1280.0,
            source_height: 720.0,
            origin_x: -100.5,
            origin_y: 200.25,
        });
        assert_eq!(
            map_control_input_event_for_target_geometry(
                &ControlInputEvent::MouseMove { x: 320, y: 90 },
                geometry,
            )
            .unwrap(),
            ControlInputEvent::MouseMove { x: 540, y: 380 },
        );
        assert_eq!(
            map_control_input_event_for_target_geometry(
                &ControlInputEvent::MouseMove {
                    x: i32::MIN,
                    y: i32::MAX
                },
                geometry,
            )
            .unwrap(),
            ControlInputEvent::MouseMove { x: -100, y: 920 },
        );
    }

    #[test]
    fn pointer_geometry_rejects_missing_degenerate_and_nonfinite_bounds() {
        let event = ControlInputEvent::MouseMove { x: 1, y: 2 };
        assert!(map_control_input_event_for_target_geometry(&event, None).is_err());
        let valid = ControlInputTargetGeometry {
            frame_width: 1920,
            frame_height: 1080,
            source_width: 1920.0,
            source_height: 1080.0,
            origin_x: 0.0,
            origin_y: 0.0,
        };
        for geometry in [
            ControlInputTargetGeometry {
                frame_width: 0,
                ..valid
            },
            ControlInputTargetGeometry {
                frame_height: 0,
                ..valid
            },
            ControlInputTargetGeometry {
                source_width: 0.0,
                ..valid
            },
            ControlInputTargetGeometry {
                source_height: -1.0,
                ..valid
            },
            ControlInputTargetGeometry {
                source_width: f64::NAN,
                ..valid
            },
            ControlInputTargetGeometry {
                origin_y: f64::INFINITY,
                ..valid
            },
            ControlInputTargetGeometry {
                source_width: f64::MAX,
                origin_x: f64::MAX,
                ..valid
            },
            ControlInputTargetGeometry {
                origin_x: f64::from(i32::MAX),
                ..valid
            },
            ControlInputTargetGeometry {
                origin_x: 0.1,
                source_width: 0.1,
                ..valid
            },
        ] {
            assert!(
                map_control_input_event_for_target_geometry(&event, Some(geometry)).is_err(),
                "{geometry:?}"
            );
        }
        // Single-pixel video and single-point geometry are valid endpoints.
        assert_eq!(
            map_control_input_event_for_target_geometry(
                &event,
                Some(ControlInputTargetGeometry {
                    frame_width: 1,
                    frame_height: 1,
                    source_width: 1.0,
                    source_height: 1.0,
                    origin_x: -30.0,
                    origin_y: 40.0,
                })
            )
            .unwrap(),
            ControlInputEvent::MouseMove { x: -30, y: 40 }
        );
    }

    #[test]
    fn target_geometry_leaves_all_non_mouse_move_events_unchanged_without_geometry() {
        for event in [
            ControlInputEvent::MouseButton {
                button: ControlInputButton::Left,
                pressed: true,
            },
            ControlInputEvent::MouseWheel { delta: 120 },
            ControlInputEvent::MouseHorizontalWheel { delta: -120 },
            ControlInputEvent::Key {
                key: ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            },
            ControlInputEvent::ReleaseAll,
        ] {
            assert_eq!(
                map_control_input_event_for_target_geometry(&event, None).unwrap(),
                event
            );
        }
    }

    #[test]
    fn macos_pointer_requires_negotiated_profile_for_the_selected_source() {
        let selected = mrd_ipc::MediaProfile {
            width: 3840,
            height: 2160,
            ..Default::default()
        };
        let valid = mrd_ipc::MediaProfileNegotiation {
            requested: selected.clone(),
            selected,
            status: "selected".into(),
            reason: None,
            selected_source_id: Some("macos:display:1".into()),
            selected_width: Some(3840),
            selected_height: Some(2160),
            downgrade_reason: None,
        };
        assert!(validate_macos_pointer_profile("macos:display:1", &valid).is_ok());
        assert!(validate_macos_pointer_profile("macos:display:2", &valid).is_err());
        let mut invalid = valid.clone();
        invalid.selected_source_id = None;
        assert!(validate_macos_pointer_profile("macos:display:1", &invalid).is_err());
        invalid = valid.clone();
        invalid.selected.width = 0;
        assert!(validate_macos_pointer_profile("macos:display:1", &invalid).is_err());
        invalid = valid;
        invalid.selected_height = Some(1080);
        assert!(validate_macos_pointer_profile("macos:display:1", &invalid).is_err());
    }

    #[test]
    fn authenticated_release_is_scoped_to_pointer_or_keyboard() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let mut registry = ControlInputRegistry::with_injector(SharedRecordingInputInjector {
            events: events.clone(),
        });
        let session_id = SessionId("scoped-release".to_string());
        registry
            .handle_session_event(
                &session_id,
                &ControlInputEvent::MouseButton {
                    button: ControlInputButton::Left,
                    pressed: true,
                },
            )
            .expect("button down");
        registry
            .handle_session_event(
                &session_id,
                &ControlInputEvent::Key {
                    key: ControlInputKey::VirtualKey { code: 0x41 },
                    pressed: true,
                },
            )
            .expect("key down");

        registry
            .release_session_scope(&session_id, ControlInputScope::Pointer)
            .expect("release pointer scope");
        assert_eq!(
            events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[
                InputEvent::MouseButton {
                    button: InputButton::Left,
                    pressed: true,
                },
                InputEvent::Key {
                    key: InputKey::VirtualKey(0x41),
                    pressed: true,
                },
                InputEvent::MouseButton {
                    button: InputButton::Left,
                    pressed: false,
                },
            ]
        );

        registry
            .release_session_scope(&session_id, ControlInputScope::Keyboard)
            .expect("release keyboard scope");
        assert_eq!(
            events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .last(),
            Some(&InputEvent::Key {
                key: InputKey::VirtualKey(0x41),
                pressed: false,
            })
        );
    }

    #[test]
    fn shared_pressed_key_is_released_only_after_last_session_releases_it() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let mut registry = ControlInputRegistry::with_injector(SharedRecordingInputInjector {
            events: events.clone(),
        });
        let first = SessionId("first-holder".to_string());
        let second = SessionId("second-holder".to_string());
        let key_down = ControlInputEvent::Key {
            key: ControlInputKey::VirtualKey { code: 0x41 },
            pressed: true,
        };
        registry
            .handle_session_event(&first, &key_down)
            .expect("first key down");
        registry
            .handle_session_event(&second, &key_down)
            .expect("second key down");
        registry
            .release_session_scope(&first, ControlInputScope::Keyboard)
            .expect("release first holder");
        assert_eq!(
            events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1
        );

        registry
            .release_session_scope(&second, ControlInputScope::Keyboard)
            .expect("release final holder");
        assert_eq!(
            events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_slice(),
            &[
                InputEvent::Key {
                    key: InputKey::VirtualKey(0x41),
                    pressed: true,
                },
                InputEvent::Key {
                    key: InputKey::VirtualKey(0x41),
                    pressed: false,
                },
            ]
        );
    }
}
