//! Quartz input injection for the logged-in macOS desktop.
//!
//! The control protocol uses Windows virtual-key identifiers. They describe
//! physical US/ANSI key positions in the current client, so they must be mapped
//! to Carbon key codes, not passed straight through to Quartz. The host's active
//! keyboard layout supplies characters; Unicode text and unsupported system
//! keys are not part of this protocol.

use crate::{InputButton, InputError, InputEvent, InputInjector, InputKey};
use core_graphics::{
    event::{
        CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField, KeyCode,
        ScrollEventUnit,
    },
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
};
use objc2_app_kit::NSEvent;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

const ACCESSIBILITY_REQUIRED: &str = "macOS Accessibility permission is required for mrd-service; enable the running service in System Settings > Privacy & Security > Accessibility";

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    // Apple's Boolean return type is an unsigned byte, not a C99 bool.
    fn AXIsProcessTrusted() -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
}

/// Checks permission without presenting a prompt or changing TCC settings.
/// Read on every call so permission changes are reflected by availability.
pub fn is_accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 && CGPreflightPostEventAccess() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosInputCommand {
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: u32,
        pressed: bool,
    },
    Scroll {
        vertical: i32,
        horizontal: i32,
    },
    Key {
        virtual_key: u16,
        key_code: u16,
        pressed: bool,
    },
}

#[derive(Debug, Clone, Default)]
struct PressedInput {
    buttons: BTreeSet<u32>,
    virtual_keys: BTreeSet<u16>,
    caps_lock: Option<bool>,
    pointer_position: Option<CGPoint>,
    mouse_clicks: BTreeMap<u32, MouseClick>,
    next_mouse_event_number: i64,
}

#[derive(Debug, Clone)]
struct MouseClick {
    position: CGPoint,
    pressed_at: Instant,
    count: i64,
    event_number: i64,
    dragged: bool,
}

fn close_click_positions(first: CGPoint, second: CGPoint) -> bool {
    (first.x - second.x).abs() <= 4.0 && (first.y - second.y).abs() <= 4.0
}

impl PressedInput {
    fn apply(&mut self, command: MacosInputCommand) {
        match command {
            MacosInputCommand::MouseMove { x, y } => {
                let position = CGPoint::new(f64::from(x), f64::from(y));
                self.pointer_position = Some(position);
                for button in &self.buttons {
                    if let Some(click) = self.mouse_clicks.get_mut(button) {
                        click.dragged |= !close_click_positions(click.position, position);
                    }
                }
            }
            MacosInputCommand::MouseButton { button, pressed } => {
                if pressed {
                    self.buttons.insert(button);
                } else {
                    self.buttons.remove(&button);
                }
            }
            MacosInputCommand::Key {
                virtual_key,
                pressed,
                ..
            } => {
                if pressed {
                    // Caps Lock toggles only on a new down transition, not on
                    // autorepeat or on the corresponding release event.
                    if self.virtual_keys.insert(virtual_key) && virtual_key == 0x14 {
                        self.caps_lock = Some(!self.caps_lock.unwrap_or(false));
                    }
                } else {
                    self.virtual_keys.remove(&virtual_key);
                }
            }
            _ => {}
        }
    }

    fn mouse_click_metadata(
        &mut self,
        button: u32,
        pressed: bool,
        position: CGPoint,
        now: Instant,
        double_click_interval: Duration,
    ) -> (i64, i64) {
        if pressed {
            let count = self
                .mouse_clicks
                .get(&button)
                .filter(|previous| {
                    !previous.dragged
                        && close_click_positions(previous.position, position)
                        && now.saturating_duration_since(previous.pressed_at)
                            <= double_click_interval
                })
                .map_or(1, |previous| previous.count.saturating_add(1));
            self.next_mouse_event_number = self.next_mouse_event_number % i64::from(i32::MAX) + 1;
            self.mouse_clicks.insert(
                button,
                MouseClick {
                    position,
                    pressed_at: now,
                    count,
                    event_number: self.next_mouse_event_number,
                    dragged: false,
                },
            );
        }
        self.mouse_clicks
            .get(&button)
            .map_or((1, 0), |click| (click.count, click.event_number))
    }

    fn modifier_flags(&self) -> CGEventFlags {
        let mut flags = CGEventFlags::empty();
        for key in &self.virtual_keys {
            flags |= match key {
                0x10 | 0xa0 | 0xa1 => CGEventFlags::CGEventFlagShift,
                0x11 | 0xa2 | 0xa3 => CGEventFlags::CGEventFlagControl,
                0x12 | 0xa4 | 0xa5 => CGEventFlags::CGEventFlagAlternate,
                0x5b | 0x5c => CGEventFlags::CGEventFlagCommand,
                _ => CGEventFlags::empty(),
            };
        }
        if self.caps_lock == Some(true) {
            flags |= CGEventFlags::CGEventFlagAlphaShift;
        }
        flags
    }

    fn motion_type_and_button(&self) -> (CGEventType, u32) {
        match self.buttons.first().copied() {
            Some(0) => (CGEventType::LeftMouseDragged, 0),
            Some(1) => (CGEventType::RightMouseDragged, 1),
            Some(button) => (CGEventType::OtherMouseDragged, button),
            None => (CGEventType::MouseMoved, 0),
        }
    }
}

/// Held keys/buttons are updated only after an event has been posted. Callers
/// release them through normal key-up/button-up events (including ReleaseAll)
/// when a session ends; no input is posted by construction or destruction.
#[derive(Debug, Default)]
pub struct MacosInputInjector {
    pressed: PressedInput,
}

impl MacosInputInjector {
    pub fn new() -> Self {
        Self::default()
    }

    fn inject_with_permission(
        &mut self,
        event: &InputEvent,
        permission_check: impl Fn() -> bool,
    ) -> Result<(), InputError> {
        require_permission(permission_check())?;
        let command = map_macos_input(event)?;
        let source = CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|()| InputError::Platform("could not create Quartz event source".into()))?;
        let mut next = self.pressed.clone();
        if next.caps_lock.is_none() {
            // Start with the desktop's lock state. A private source prevents
            // unrelated locally held modifiers from leaking into remote input.
            let desktop_source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .map_err(|()| InputError::Platform("could not read desktop input state".into()))?;
            let desktop_event = CGEvent::new(desktop_source)
                .map_err(|()| InputError::Platform("could not read desktop input state".into()))?;
            next.caps_lock = Some(
                desktop_event
                    .get_flags()
                    .contains(CGEventFlags::CGEventFlagAlphaShift),
            );
        }
        next.apply(command);
        let native_event = create_native_event(source, command, &self.pressed, &mut next, None)?;
        // Do not post an event if permission was revoked while it was built.
        require_permission(permission_check())?;
        native_event.post(CGEventTapLocation::HID);
        self.pressed = next;
        Ok(())
    }
}

impl InputInjector for MacosInputInjector {
    fn is_available(&self) -> bool {
        is_accessibility_trusted()
    }

    fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
        self.inject_with_permission(event, is_accessibility_trusted)
    }

    fn reset_idle_state(&mut self) {
        if self.pressed.buttons.is_empty() && self.pressed.virtual_keys.is_empty() {
            self.pressed = PressedInput::default();
        }
    }
}

fn require_permission(trusted: bool) -> Result<(), InputError> {
    if trusted {
        Ok(())
    } else {
        Err(InputError::Unavailable(ACCESSIBILITY_REQUIRED.into()))
    }
}

fn map_macos_input(event: &InputEvent) -> Result<MacosInputCommand, InputError> {
    Ok(match *event {
        InputEvent::MouseMove { x, y } => MacosInputCommand::MouseMove { x, y },
        InputEvent::MouseButton { button, pressed } => MacosInputCommand::MouseButton {
            button: match button {
                InputButton::Left => 0,
                InputButton::Right => 1,
                InputButton::Middle => 2,
                InputButton::Other(1) => 3,
                InputButton::Other(2) => 4,
                InputButton::Other(other) => {
                    return Err(InputError::InvalidEvent(format!(
                        "unsupported macOS mouse button {other}"
                    )))
                }
            },
            pressed,
        },
        // The client sends signed pixel deltas, including trackpad deltas
        // smaller than a Windows wheel detent. Keep their magnitude and axes.
        InputEvent::MouseWheel { delta } => MacosInputCommand::Scroll {
            vertical: delta,
            horizontal: 0,
        },
        InputEvent::MouseHorizontalWheel { delta } => MacosInputCommand::Scroll {
            vertical: 0,
            horizontal: delta,
        },
        InputEvent::Key {
            key: InputKey::VirtualKey(virtual_key),
            pressed,
        } => MacosInputCommand::Key {
            virtual_key,
            key_code: macos_key_code(virtual_key)?,
            pressed,
        },
    })
}

fn mouse_event(
    source: CGEventSource,
    event_type: CGEventType,
    position: CGPoint,
    button: u32,
) -> Result<CGEvent, ()> {
    // core-graphics models only the first three button enum values. Quartz
    // supports more buttons via its numeric field; never transmute that enum.
    let cg_button = match button {
        0 => CGMouseButton::Left,
        1 => CGMouseButton::Right,
        _ => CGMouseButton::Center,
    };
    let event = CGEvent::new_mouse_event(source, event_type, position, cg_button)?;
    event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, i64::from(button));
    Ok(event)
}

fn create_native_event(
    source: CGEventSource,
    command: MacosInputCommand,
    previous: &PressedInput,
    next: &mut PressedInput,
    click_timing: Option<(Instant, Duration)>,
) -> Result<CGEvent, InputError> {
    let mut flags = next.modifier_flags();
    let result = match command {
        MacosInputCommand::MouseMove { x, y } => {
            let (event_type, button) = previous.motion_type_and_button();
            let event = mouse_event(
                source,
                event_type,
                CGPoint::new(f64::from(x), f64::from(y)),
                button,
            );
            if let (Ok(event), Some(click)) = (&event, previous.mouse_clicks.get(&button)) {
                if previous.buttons.contains(&button) {
                    event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click.count);
                    event.set_integer_value_field(
                        EventField::MOUSE_EVENT_NUMBER,
                        click.event_number,
                    );
                }
            }
            event
        }
        MacosInputCommand::MouseButton { button, pressed } => {
            // Keep clicks at the last remote position even if an immediately
            // preceding posted motion has not reached WindowServer yet. For
            // a button before any remote motion, sample the desktop cursor.
            let position = if let Some(position) = previous.pointer_position {
                position
            } else {
                let desktop_source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                    .map_err(|()| InputError::Platform("could not read cursor location".into()))?;
                CGEvent::new(desktop_source)
                    .map_err(|()| InputError::Platform("could not read cursor location".into()))?
                    .location()
            };
            let event_type = match (button, pressed) {
                (0, true) => CGEventType::LeftMouseDown,
                (0, false) => CGEventType::LeftMouseUp,
                (1, true) => CGEventType::RightMouseDown,
                (1, false) => CGEventType::RightMouseUp,
                (_, true) => CGEventType::OtherMouseDown,
                (_, false) => CGEventType::OtherMouseUp,
            };
            let event = mouse_event(source, event_type, position, button);
            if let Ok(event) = &event {
                // AppKit applications consume Quartz click counts directly.
                // Use the user's double-click interval and keep the event
                // number stable between a button's matching down/up pair.
                let (now, interval) = click_timing.unwrap_or_else(|| {
                    let interval = Duration::try_from_secs_f64(NSEvent::doubleClickInterval())
                        .unwrap_or(Duration::from_millis(500));
                    (Instant::now(), interval)
                });
                let (count, number) =
                    next.mouse_click_metadata(button, pressed, position, now, interval);
                event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, count);
                event.set_integer_value_field(EventField::MOUSE_EVENT_NUMBER, number);
            }
            event
        }
        MacosInputCommand::Scroll {
            vertical,
            horizontal,
        } => {
            let event = CGEvent::new_scroll_event(
                source,
                ScrollEventUnit::PIXEL,
                2,
                vertical,
                horizontal,
                0,
            );
            if let (Ok(event), Some(position)) = (&event, previous.pointer_position) {
                // A queued motion may not have reached WindowServer yet.
                // Target the same last posted remote position as clicks.
                event.set_location(position);
            }
            event
        }
        MacosInputCommand::Key {
            virtual_key,
            key_code,
            pressed,
        } => {
            if (0x60..=0x6f).contains(&virtual_key) || virtual_key == 0x0c {
                flags |= CGEventFlags::CGEventFlagNumericPad;
            }
            let event = CGEvent::new_keyboard_event(source, key_code, pressed);
            if let Ok(event) = &event {
                if matches!(virtual_key, 0x10..=0x12 | 0x14 | 0x5b..=0x5c | 0xa0..=0xa5) {
                    event.set_type(CGEventType::FlagsChanged);
                } else {
                    event.set_integer_value_field(
                        EventField::KEYBOARD_EVENT_AUTOREPEAT,
                        i64::from(pressed && previous.virtual_keys.contains(&virtual_key)),
                    );
                }
            }
            event
        }
    };
    let event =
        result.map_err(|()| InputError::Platform("could not create Quartz input event".into()))?;
    event.set_flags(flags);
    Ok(event)
}

fn macos_key_code(virtual_key: u16) -> Result<u16, InputError> {
    // These position codes are defined in Apple's HIToolbox/Events.h. Table
    // indices are Windows VK_A..VK_Z, VK_0..VK_9, VK_NUMPAD0..9 and VK_F1..20.
    const LETTERS: [u16; 26] = [
        0x00, 0x0b, 0x08, 0x02, 0x0e, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2e, 0x2d, 0x1f,
        0x23, 0x0c, 0x0f, 0x01, 0x11, 0x20, 0x09, 0x0d, 0x07, 0x10, 0x06,
    ];
    const DIGITS: [u16; 10] = [0x1d, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1a, 0x1c, 0x19];
    const KEYPAD: [u16; 10] = [0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5b, 0x5c];
    const FUNCTIONS: [u16; 20] = [
        KeyCode::F1,
        KeyCode::F2,
        KeyCode::F3,
        KeyCode::F4,
        KeyCode::F5,
        KeyCode::F6,
        KeyCode::F7,
        KeyCode::F8,
        KeyCode::F9,
        KeyCode::F10,
        KeyCode::F11,
        KeyCode::F12,
        KeyCode::F13,
        KeyCode::F14,
        KeyCode::F15,
        KeyCode::F16,
        KeyCode::F17,
        KeyCode::F18,
        KeyCode::F19,
        KeyCode::F20,
    ];
    Ok(match virtual_key {
        0x41..=0x5a => LETTERS[usize::from(virtual_key - 0x41)],
        0x30..=0x39 => DIGITS[usize::from(virtual_key - 0x30)],
        0x60..=0x69 => KEYPAD[usize::from(virtual_key - 0x60)],
        0x70..=0x83 => FUNCTIONS[usize::from(virtual_key - 0x70)],
        0x08 => KeyCode::DELETE,
        0x09 => KeyCode::TAB,
        0x0c => 0x47, // Clear
        0x0d => KeyCode::RETURN,
        0x10 | 0xa0 => KeyCode::SHIFT,
        0x11 | 0xa2 => KeyCode::CONTROL,
        0x12 | 0xa4 => KeyCode::OPTION,
        0x14 => KeyCode::CAPS_LOCK,
        0x1b => KeyCode::ESCAPE,
        0x20 => KeyCode::SPACE,
        0x21 => KeyCode::PAGE_UP,
        0x22 => KeyCode::PAGE_DOWN,
        0x23 => KeyCode::END,
        0x24 => KeyCode::HOME,
        0x25 => KeyCode::LEFT_ARROW,
        0x26 => KeyCode::UP_ARROW,
        0x27 => KeyCode::RIGHT_ARROW,
        0x28 => KeyCode::DOWN_ARROW,
        0x2e => KeyCode::FORWARD_DELETE,
        0x2f => KeyCode::HELP,
        0x5b => KeyCode::COMMAND,
        0x5c => KeyCode::RIGHT_COMMAND,
        0x5d => 0x6e, // Contextual Menu
        0x6a => 0x43, // Keypad Multiply
        0x6b => 0x45, // Keypad Plus
        0x6d => 0x4e, // Keypad Minus
        0x6e => 0x41, // Keypad Decimal
        0x6f => 0x4b, // Keypad Divide
        0xa1 => KeyCode::RIGHT_SHIFT,
        0xa3 => KeyCode::RIGHT_CONTROL,
        0xa5 => KeyCode::RIGHT_OPTION,
        0xba => 0x29, // Semicolon
        0xbb => 0x18, // Equal
        0xbc => 0x2b, // Comma
        0xbd => 0x1b, // Minus
        0xbe => 0x2f, // Period
        0xbf => 0x2c, // Slash
        0xc0 => 0x32, // Backquote
        0xdb => 0x21, // Left Bracket
        0xdc => 0x2a, // Backslash
        0xdd => 0x1e, // Right Bracket
        0xde => 0x27, // Quote
        _ => {
            return Err(InputError::InvalidEvent(format!(
                "Windows virtual key 0x{virtual_key:04x} has no supported macOS equivalent"
            )))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(virtual_key: u16, pressed: bool) -> MacosInputCommand {
        map_macos_input(&InputEvent::Key {
            key: InputKey::VirtualKey(virtual_key),
            pressed,
        })
        .expect("supported key")
    }

    fn native(command: MacosInputCommand, previous: &PressedInput) -> (CGEvent, PressedInput) {
        let source = CGEventSource::new(CGEventSourceStateID::Private).expect("event source");
        let mut next = previous.clone();
        next.apply(command);
        let now = previous
            .mouse_clicks
            .values()
            .map(|click| click.pressed_at)
            .max()
            .unwrap_or_else(Instant::now)
            + Duration::from_millis(1);
        let event = create_native_event(
            source,
            command,
            previous,
            &mut next,
            Some((now, Duration::from_millis(500))),
        )
        .expect("native event");
        // No tests post events to the user's desktop.
        (event, next)
    }

    #[test]
    fn protocol_keys_translate_to_mac_codes_instead_of_passthrough() {
        for (virtual_key, expected) in [
            (0x41, 0x00),
            (0x5a, 0x06),
            (0x30, 0x1d),
            (0x39, 0x19),
            (0x60, 0x52),
            (0x69, 0x5c),
            (0x6a, 0x43),
            (0x6f, 0x4b),
            (0x70, KeyCode::F1),
            (0x83, KeyCode::F20),
            (0x08, KeyCode::DELETE),
            (0x2e, KeyCode::FORWARD_DELETE),
            (0x11, KeyCode::CONTROL),
            (0xa3, KeyCode::RIGHT_CONTROL),
            (0x5b, KeyCode::COMMAND),
            (0x5c, KeyCode::RIGHT_COMMAND),
            (0xba, 0x29),
            (0xde, 0x27),
        ] {
            assert_eq!(macos_key_code(virtual_key).expect("map"), expected);
        }
        for unsupported in [0, 0x13, 0x2c, 0x2d, 0x84, 0x90, 0x91, 0xffff] {
            assert!(matches!(
                macos_key_code(unsupported),
                Err(InputError::InvalidEvent(_))
            ));
        }
    }

    #[test]
    fn all_letter_positions_are_unique_and_keypad_is_distinct() {
        let letters: BTreeSet<_> = (0x41..=0x5a)
            .map(|code| macos_key_code(code).expect("letter"))
            .collect();
        assert_eq!(letters.len(), 26);
        for digit in 0..10 {
            assert_ne!(macos_key_code(0x30 + digit), macos_key_code(0x60 + digit));
        }
    }

    #[test]
    fn modifiers_are_attached_to_keys_pointer_and_release_events() {
        let (_, mut state) = native(key(0x10, true), &PressedInput::default());
        let (_, next) = native(key(0x5b, true), &state);
        state = next;
        let (letter, next) = native(key(0x41, true), &state);
        assert!(letter.get_flags().contains(CGEventFlags::CGEventFlagShift));
        assert!(letter
            .get_flags()
            .contains(CGEventFlags::CGEventFlagCommand));
        assert_eq!(
            letter.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
            0
        );
        state = next;
        let (pointer, _) = native(MacosInputCommand::MouseMove { x: -20, y: 50 }, &state);
        assert!(pointer.get_flags().contains(CGEventFlags::CGEventFlagShift));
        assert_eq!((pointer.location().x, pointer.location().y), (-20.0, 50.0));
        let (release, next) = native(key(0x10, false), &state);
        assert_eq!(release.get_type() as u32, CGEventType::FlagsChanged as u32);
        assert!(!release.get_flags().contains(CGEventFlags::CGEventFlagShift));
        assert!(release
            .get_flags()
            .contains(CGEventFlags::CGEventFlagCommand));
        let (release, _) = native(key(0x5b, false), &next);
        assert!(release.get_flags().is_empty());
    }

    #[test]
    fn caps_lock_toggles_once_per_down_transition() {
        let mut state = PressedInput::default();
        state.apply(key(0x14, true));
        assert_eq!(state.caps_lock, Some(true));
        state.apply(key(0x14, true));
        state.apply(key(0x14, false));
        assert_eq!(state.caps_lock, Some(true));
        state.apply(key(0x14, true));
        assert_eq!(state.caps_lock, Some(false));
    }

    #[test]
    fn modifier_release_keeps_other_side_held_and_autorepeat_is_marked() {
        let (_, state) = native(key(0xa0, true), &PressedInput::default());
        let (_, state) = native(key(0xa1, true), &state);
        let (event, state) = native(key(0xa0, false), &state);
        assert!(event.get_flags().contains(CGEventFlags::CGEventFlagShift));
        let (_, state) = native(key(0x41, true), &state);
        let (repeat, state) = native(key(0x41, true), &state);
        assert_eq!(
            repeat.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT),
            1
        );
        let (release, _) = native(key(0x41, false), &state);
        assert_eq!(
            release.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT),
            0
        );
    }

    #[test]
    fn buttons_use_native_numbers_and_motion_becomes_dragging_until_release() {
        for (button, number, down_type, drag_type) in [
            (
                InputButton::Left,
                0,
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseDragged,
            ),
            (
                InputButton::Right,
                1,
                CGEventType::RightMouseDown,
                CGEventType::RightMouseDragged,
            ),
            (
                InputButton::Middle,
                2,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseDragged,
            ),
            (
                InputButton::Other(1),
                3,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseDragged,
            ),
            (
                InputButton::Other(2),
                4,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseDragged,
            ),
        ] {
            let down = map_macos_input(&InputEvent::MouseButton {
                button,
                pressed: true,
            })
            .expect("button");
            let (event, state) = native(down, &PressedInput::default());
            let down_number = event.get_integer_value_field(EventField::MOUSE_EVENT_NUMBER);
            assert!(down_number > 0);
            assert_eq!(event.get_type() as u32, down_type as u32);
            assert_eq!(
                event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER),
                number
            );
            let motion = MacosInputCommand::MouseMove { x: 25, y: 75 };
            let (event, _) = native(motion, &state);
            assert_eq!(event.get_type() as u32, drag_type as u32);
            assert_eq!(
                event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER),
                number
            );
            let up = map_macos_input(&InputEvent::MouseButton {
                button,
                pressed: false,
            })
            .expect("button");
            let (release, state) = native(up, &state);
            assert_eq!(
                release.get_integer_value_field(EventField::MOUSE_EVENT_NUMBER),
                down_number
            );
            let (event, _) = native(motion, &state);
            assert_eq!(event.get_type() as u32, CGEventType::MouseMoved as u32);
        }
        assert!(matches!(
            map_macos_input(&InputEvent::MouseButton {
                button: InputButton::Other(0),
                pressed: true
            }),
            Err(InputError::InvalidEvent(_))
        ));
    }

    #[test]
    fn clicks_use_last_remote_position_and_count_successive_clicks() {
        let (_, state) = native(
            MacosInputCommand::MouseMove { x: -125, y: 75 },
            &PressedInput::default(),
        );
        let down = MacosInputCommand::MouseButton {
            button: 0,
            pressed: true,
        };
        let up = MacosInputCommand::MouseButton {
            button: 0,
            pressed: false,
        };
        let (first, state) = native(down, &state);
        assert_eq!((first.location().x, first.location().y), (-125.0, 75.0));
        assert_eq!(
            first.get_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE),
            1
        );
        let (_, state) = native(up, &state);
        let (second, state) = native(down, &state);
        assert_eq!(
            second.get_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE),
            2
        );
        let (release, _) = native(up, &state);
        assert_eq!(
            release.get_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE),
            2
        );
    }

    #[test]
    fn click_counts_reset_outside_interval_or_after_pointer_drag() {
        let mut state = PressedInput::default();
        let position = CGPoint::new(10.0, 20.0);
        let now = Instant::now();
        let interval = Duration::from_millis(300);
        let first = state.mouse_click_metadata(0, true, position, now, interval);
        assert_eq!(first.0, 1);
        assert_eq!(
            state.mouse_click_metadata(0, false, position, now, interval),
            first
        );
        let second = state.mouse_click_metadata(
            0,
            true,
            position,
            now + Duration::from_millis(250),
            interval,
        );
        assert_eq!(second.0, 2);
        assert_ne!(second.1, first.1);
        assert_eq!(
            state
                .mouse_click_metadata(
                    0,
                    true,
                    position,
                    now + Duration::from_millis(600),
                    interval,
                )
                .0,
            1
        );
        state.apply(MacosInputCommand::MouseButton {
            button: 0,
            pressed: true,
        });
        state.apply(MacosInputCommand::MouseMove { x: 50, y: 80 });
        state.apply(MacosInputCommand::MouseMove { x: 10, y: 20 });
        state.apply(MacosInputCommand::MouseButton {
            button: 0,
            pressed: false,
        });
        assert_eq!(
            state
                .mouse_click_metadata(
                    0,
                    true,
                    position,
                    now + Duration::from_millis(650),
                    interval,
                )
                .0,
            1
        );
        assert_eq!(
            state
                .mouse_click_metadata(
                    0,
                    true,
                    CGPoint::new(50.0, 80.0),
                    now + Duration::from_millis(700),
                    interval,
                )
                .0,
            1
        );
    }

    #[test]
    fn pixel_scroll_preserves_small_signed_deltas_and_separate_axes() {
        for (input, vertical, horizontal) in [
            (InputEvent::MouseWheel { delta: 1 }, 1, 0),
            (InputEvent::MouseWheel { delta: -120 }, -120, 0),
            (InputEvent::MouseHorizontalWheel { delta: -3 }, 0, -3),
        ] {
            let command = map_macos_input(&input).expect("scroll");
            assert_eq!(
                command,
                MacosInputCommand::Scroll {
                    vertical,
                    horizontal
                }
            );
            let (event, _) = native(command, &PressedInput::default());
            assert_eq!(
                event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1),
                i64::from(vertical)
            );
            assert_eq!(
                event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2),
                i64::from(horizontal)
            );
        }
    }

    #[test]
    fn scroll_uses_last_remote_position_without_dispatching_motion() {
        let (_, state) = native(
            MacosInputCommand::MouseMove { x: -1500, y: 375 },
            &PressedInput::default(),
        );
        for input in [
            InputEvent::MouseWheel { delta: 9 },
            InputEvent::MouseHorizontalWheel { delta: -3 },
        ] {
            let (event, _) = native(map_macos_input(&input).expect("scroll"), &state);
            assert_eq!((event.location().x, event.location().y), (-1500.0, 375.0));
            assert_eq!(event.get_type() as u32, CGEventType::ScrollWheel as u32);
        }
    }

    #[test]
    fn idle_reset_clears_pointer_click_and_lock_context_between_sessions() {
        let mut injector = MacosInputInjector::new();
        injector.pressed.pointer_position = Some(CGPoint::new(-125.0, 75.0));
        injector.pressed.caps_lock = Some(true);
        injector.pressed.mouse_click_metadata(
            0,
            true,
            CGPoint::new(-125.0, 75.0),
            Instant::now(),
            Duration::from_millis(500),
        );
        injector.reset_idle_state();
        assert!(injector.pressed.pointer_position.is_none());
        assert!(injector.pressed.mouse_clicks.is_empty());
        assert!(injector.pressed.caps_lock.is_none());
        assert_eq!(injector.pressed.next_mouse_event_number, 0);
        let (first, _) = native(
            MacosInputCommand::MouseButton {
                button: 0,
                pressed: true,
            },
            &injector.pressed,
        );
        assert_eq!(
            first.get_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE),
            1
        );
    }

    #[test]
    fn idle_reset_preserves_held_input_until_releases_succeed() {
        let mut injector = MacosInputInjector::new();
        injector.pressed.apply(key(0x10, true));
        injector.pressed.apply(MacosInputCommand::MouseButton {
            button: 0,
            pressed: true,
        });
        injector.pressed.pointer_position = Some(CGPoint::new(100.0, 200.0));
        injector.reset_idle_state();
        assert!(injector.pressed.virtual_keys.contains(&0x10));
        assert!(injector.pressed.buttons.contains(&0));
        assert!(injector.pressed.pointer_position.is_some());
        injector.pressed.apply(key(0x10, false));
        injector.pressed.apply(MacosInputCommand::MouseButton {
            button: 0,
            pressed: false,
        });
        injector.reset_idle_state();
        assert!(injector.pressed.pointer_position.is_none());
    }

    #[test]
    fn permission_denial_is_actionable_and_does_not_change_held_state() {
        let mut injector = MacosInputInjector::new();
        injector.pressed.apply(key(0x10, true));
        let error = injector
            .inject_with_permission(
                &InputEvent::Key {
                    key: InputKey::VirtualKey(0x10),
                    pressed: false,
                },
                || false,
            )
            .expect_err("must deny before creating or posting events");
        assert!(
            matches!(error, InputError::Unavailable(ref reason) if reason.contains("Accessibility"))
        );
        assert!(injector.pressed.virtual_keys.contains(&0x10));
    }

    #[test]
    fn permission_is_rechecked_before_posting_and_can_recover() {
        use std::cell::Cell;
        let mut injector = MacosInputInjector::new();
        let checks = Cell::new(0);
        let result = injector.inject_with_permission(&InputEvent::MouseMove { x: 1, y: 2 }, || {
            checks.set(checks.get() + 1);
            checks.get() == 1
        });
        assert!(matches!(result, Err(InputError::Unavailable(_))));
        assert_eq!(checks.get(), 2);
        assert!(injector.pressed.buttons.is_empty());
        assert!(injector.pressed.virtual_keys.is_empty());
        assert!(require_permission(true).is_ok());
    }
}
