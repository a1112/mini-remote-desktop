use mrd_input::{
    InputButton, InputError, InputEvent, InputInjector, InputKey, TrackedInputInjector,
};
use mrd_ipc::{
    ControlChannelLaneSnapshot, ControlChannelReliability, ControlChannelSnapshot,
    ControlInputButton, ControlInputEvent, ControlInputKey, ControlInputLane,
};
use mrd_proto::SessionId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlInputResult {
    pub lane: ControlInputLane,
    pub event_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlInputTargetGeometry {
    pub frame_width: u32,
    pub frame_height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub origin_x: i32,
    pub origin_y: i32,
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

pub struct ControlInputRegistry {
    injector: TrackedInputInjector<Box<dyn InputInjector>>,
    reliable: ControlLaneCounters,
    realtime: ControlLaneCounters,
}

impl ControlInputRegistry {
    pub fn default_for_platform() -> Self {
        #[cfg(windows)]
        let injector: Box<dyn InputInjector> =
            Box::new(mrd_input::windows::WindowsSendInputInjector::new());

        #[cfg(not(windows))]
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
            injector: TrackedInputInjector::new(Box::new(injector)),
            reliable: ControlLaneCounters::default(),
            realtime: ControlLaneCounters::default(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.injector.is_available()
    }

    pub fn handle_event(
        &mut self,
        event: &ControlInputEvent,
    ) -> Result<ControlInputResult, InputError> {
        let lane = input_lane(event);
        counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane).accepted_messages += 1;

        let result: Result<u32, InputError> = match event {
            ControlInputEvent::ReleaseAll => self
                .injector
                .release_all()
                .map(|released| released.len() as u32),
            event => input_event_from_ipc(event)
                .and_then(|input| self.injector.inject(&input))
                .map(|()| 1),
        };

        match result {
            Ok(event_count) => {
                counter_for_lane_mut(&mut self.reliable, &mut self.realtime, lane)
                    .injected_messages += u64::from(event_count);
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
}

impl Default for ControlInputRegistry {
    fn default() -> Self {
        Self::default_for_platform()
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

pub fn map_control_input_event_for_target_geometry(
    event: &ControlInputEvent,
    geometry: Option<ControlInputTargetGeometry>,
) -> ControlInputEvent {
    let Some(geometry) = geometry else {
        return event.clone();
    };
    match *event {
        ControlInputEvent::MouseMove { x, y } => {
            let x = scale_target_coordinate(
                x,
                geometry.frame_width,
                geometry.source_width,
                geometry.origin_x,
            );
            let y = scale_target_coordinate(
                y,
                geometry.frame_height,
                geometry.source_height,
                geometry.origin_y,
            );
            ControlInputEvent::MouseMove { x, y }
        }
        _ => event.clone(),
    }
}

fn scale_target_coordinate(
    coordinate: i32,
    frame_extent: u32,
    source_extent: u32,
    origin: i32,
) -> i32 {
    if frame_extent == 0 || source_extent == 0 {
        return coordinate;
    }
    let scaled = i64::from(coordinate) * i64::from(source_extent) / i64::from(frame_extent);
    let max_source = i64::from(source_extent.saturating_sub(1));
    let bounded = scaled.clamp(0, max_source) + i64::from(origin);
    bounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
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
    fn target_geometry_scales_frame_mouse_move_to_capture_source_coordinates() {
        let event = map_control_input_event_for_target_geometry(
            &ControlInputEvent::MouseMove { x: 640, y: 360 },
            Some(ControlInputTargetGeometry {
                frame_width: 1280,
                frame_height: 720,
                source_width: 2560,
                source_height: 1440,
                origin_x: 0,
                origin_y: 0,
            }),
        );

        assert_eq!(event, ControlInputEvent::MouseMove { x: 1280, y: 720 });
    }

    #[test]
    fn target_geometry_adds_display_origin_and_clamps_to_source_bounds() {
        let event = map_control_input_event_for_target_geometry(
            &ControlInputEvent::MouseMove { x: 1280, y: 720 },
            Some(ControlInputTargetGeometry {
                frame_width: 1280,
                frame_height: 720,
                source_width: 2560,
                source_height: 1440,
                origin_x: 1920,
                origin_y: -120,
            }),
        );

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
                    source_width: 2560,
                    source_height: 1440,
                    origin_x: 1920,
                    origin_y: 0,
                }),
            ),
            event
        );
    }
}
