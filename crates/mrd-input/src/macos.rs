//! CoreGraphics injection in desktop point coordinates. The wire keyboard
//! contract remains Windows virtual keys, as in R-ShareMouse's canonical map.
use crate::{InputButton, InputError, InputEvent, InputInjector, InputKey};
use core_graphics::{
    event::{
        CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
        ScrollEventUnit,
    },
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
}

/// Checks the actual service process's Accessibility permission without prompting.
pub fn can_post_events() -> bool {
    unsafe { CGPreflightPostEventAccess() }
}

/// Resolve current native point bounds without enumerating ScreenCaptureKit on
/// each pointer event. Display pixels and CGEvent points differ on Retina Macs.
pub fn capture_bounds(
    source_id: Option<&str>,
) -> Result<core_graphics::geometry::CGRect, InputError> {
    use core_foundation::{array::CFArray, dictionary::CFDictionary, string::CFString};
    use core_graphics::{
        display::CGDisplay, geometry::CGRect, window::create_description_from_array,
    };
    let parse = |text: &str| {
        u32::from_str_radix(
            text.strip_prefix("0x").unwrap_or(text),
            if text.starts_with("0x") { 16 } else { 10 },
        )
        .map_err(|_| InputError::InvalidEvent("invalid macOS capture source ID".into()))
    };
    let bounds = match source_id {
        None => CGDisplay::main().bounds(),
        Some(source) if source.starts_with("macos:display:") => {
            let id = parse(&source[14..])?;
            if !CGDisplay::active_displays()
                .map_err(|_| InputError::Unavailable("display enumeration failed".into()))?
                .contains(&id)
            {
                return Err(InputError::Unavailable(
                    "capture display disconnected".into(),
                ));
            }
            CGDisplay::new(id).bounds()
        }
        Some(source) if source.starts_with("macos:window:") => {
            let windows =
                create_description_from_array(CFArray::from_copyable(&[parse(&source[13..])?]))
                    .ok_or_else(|| InputError::Unavailable("capture window unavailable".into()))?;
            let window = windows
                .get(0)
                .ok_or_else(|| InputError::Unavailable("capture window closed".into()))?;
            let value = window
                .find(CFString::new("kCGWindowBounds"))
                .ok_or_else(|| InputError::Unavailable("window bounds unavailable".into()))?;
            let dictionary = value
                .downcast::<CFDictionary>()
                .ok_or_else(|| InputError::Unavailable("invalid window bounds".into()))?;
            CGRect::from_dict_representation(&dictionary)
                .ok_or_else(|| InputError::Unavailable("invalid window rectangle".into()))?
        }
        Some(_) => {
            return Err(InputError::InvalidEvent(
                "unsupported macOS input capture source".into(),
            ))
        }
    };
    if !bounds.origin.x.is_finite()
        || !bounds.origin.y.is_finite()
        || !bounds.size.width.is_finite()
        || !bounds.size.height.is_finite()
        || bounds.size.width <= 0.0
        || bounds.size.height <= 0.0
    {
        return Err(InputError::Unavailable(
            "capture source has invalid bounds".into(),
        ));
    }
    Ok(bounds)
}

#[derive(Default)]
pub struct MacosInputInjector {
    buttons: HashSet<InputButton>,
    keys: HashSet<u16>,
    last_click: Option<(InputButton, Instant, CGPoint, i64)>,
    wheel_remainder: [i32; 2],
}

impl MacosInputInjector {
    pub fn new() -> Self {
        Self::default()
    }

    fn source() -> Result<CGEventSource, InputError> {
        CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|_| InputError::Platform("cannot create CoreGraphics event source".into()))
    }

    fn position() -> Result<CGPoint, InputError> {
        CGEvent::new(Self::source()?)
            .map(|event| event.location())
            .map_err(|_| InputError::Platform("cannot read pointer location".into()))
    }

    fn flags(&self) -> CGEventFlags {
        let mut flags = CGEventFlags::empty();
        for (keys, flag) in [
            (&[0x10, 0xA0, 0xA1][..], CGEventFlags::CGEventFlagShift),
            (&[0x11, 0xA2, 0xA3][..], CGEventFlags::CGEventFlagControl),
            (&[0x12, 0xA4, 0xA5][..], CGEventFlags::CGEventFlagAlternate),
            (&[0x5B, 0x5C][..], CGEventFlags::CGEventFlagCommand),
        ] {
            if keys.iter().any(|key| self.keys.contains(key)) {
                flags |= flag;
            }
        }
        flags
    }
}

impl InputInjector for MacosInputInjector {
    fn is_available(&self) -> bool {
        can_post_events()
    }

    fn inject(&mut self, input: &InputEvent) -> Result<(), InputError> {
        if !self.is_available() {
            return Err(InputError::Unavailable(
                "Enable Accessibility for Rdesk Service in System Settings, then restart the service".into(),
            ));
        }
        let event = match *input {
            InputEvent::MouseMove { x, y } => {
                let button = [InputButton::Left, InputButton::Right, InputButton::Middle]
                    .into_iter()
                    .find(|button| self.buttons.contains(button))
                    .or_else(|| self.buttons.iter().copied().next());
                let (kind, native, number) = drag_spec(button)?;
                let event = CGEvent::new_mouse_event(
                    Self::source()?,
                    kind,
                    CGPoint::new(x as f64, y as f64),
                    native,
                );
                if let Ok(event) = &event {
                    event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, number);
                }
                event
            }
            InputEvent::MouseButton { button, pressed } => {
                let (native, number) = button_spec(button)?;
                let kind = match (button, pressed) {
                    (InputButton::Left, true) => CGEventType::LeftMouseDown,
                    (InputButton::Left, false) => CGEventType::LeftMouseUp,
                    (InputButton::Right, true) => CGEventType::RightMouseDown,
                    (InputButton::Right, false) => CGEventType::RightMouseUp,
                    (_, true) => CGEventType::OtherMouseDown,
                    (_, false) => CGEventType::OtherMouseUp,
                };
                let position = Self::position()?;
                let event = CGEvent::new_mouse_event(Self::source()?, kind, position, native);
                if let Ok(event) = &event {
                    let count = if pressed {
                        let count = match self.last_click {
                            Some((last, at, point, count))
                                if last == button
                                    && at.elapsed() < Duration::from_millis(500)
                                    && (point.x - position.x).abs() <= 4.0
                                    && (point.y - position.y).abs() <= 4.0 =>
                            {
                                (count % 2) + 1
                            }
                            _ => 1,
                        };
                        self.last_click = Some((button, Instant::now(), position, count));
                        self.buttons.insert(button);
                        count
                    } else {
                        self.buttons.remove(&button);
                        self.last_click
                            .filter(|click| click.0 == button)
                            .map_or(1, |click| click.3)
                    };
                    event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, number);
                    event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, count);
                }
                event
            }
            InputEvent::Key {
                key: InputKey::VirtualKey(key),
                pressed,
            } => {
                let native = virtual_key_to_macos(key).ok_or_else(|| {
                    InputError::InvalidEvent(format!("unsupported macOS virtual key {key:#x}"))
                })?;
                let event = CGEvent::new_keyboard_event(Self::source()?, native, pressed);
                if let Ok(event) = &event {
                    let repeat = pressed && self.keys.contains(&key);
                    event.set_integer_value_field(
                        EventField::KEYBOARD_EVENT_AUTOREPEAT,
                        i64::from(repeat),
                    );
                    if pressed {
                        self.keys.insert(key);
                    } else {
                        self.keys.remove(&key);
                    }
                }
                event
            }
            InputEvent::MouseWheel { delta } | InputEvent::MouseHorizontalWheel { delta } => {
                let horizontal = matches!(input, InputEvent::MouseHorizontalWheel { .. });
                let slot = usize::from(horizontal);
                let accumulated = self.wheel_remainder[slot].saturating_add(delta);
                let lines = accumulated / 120;
                self.wheel_remainder[slot] = accumulated % 120;
                if lines == 0 {
                    return Ok(());
                }
                CGEvent::new_scroll_event(
                    Self::source()?,
                    ScrollEventUnit::LINE,
                    2,
                    if horizontal { 0 } else { lines },
                    if horizontal { lines } else { 0 },
                    0,
                )
            }
        }
        .map_err(|_| InputError::Platform("cannot create CoreGraphics input event".into()))?;
        event.set_flags(self.flags());
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, 0x4D524449);
        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}

fn button_spec(button: InputButton) -> Result<(CGMouseButton, i64), InputError> {
    match button {
        InputButton::Left => Ok((CGMouseButton::Left, 0)),
        InputButton::Right => Ok((CGMouseButton::Right, 1)),
        InputButton::Middle => Ok((CGMouseButton::Center, 2)),
        InputButton::Other(number @ 1..=29) => Ok((CGMouseButton::Center, i64::from(number) + 2)),
        _ => Err(InputError::InvalidEvent(
            "unsupported macOS mouse button".into(),
        )),
    }
}

fn drag_spec(button: Option<InputButton>) -> Result<(CGEventType, CGMouseButton, i64), InputError> {
    let kind = match button {
        None => CGEventType::MouseMoved,
        Some(InputButton::Left) => CGEventType::LeftMouseDragged,
        Some(InputButton::Right) => CGEventType::RightMouseDragged,
        Some(_) => CGEventType::OtherMouseDragged,
    };
    let (native, number) = button_spec(button.unwrap_or(InputButton::Left))?;
    Ok((kind, native, number))
}

pub fn virtual_key_to_macos(key: u16) -> Option<u16> {
    const LETTERS: [u16; 26] = [
        0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16,
        6,
    ];
    const DIGITS: [u16; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    const FUNCTION: [u16; 20] = [
        122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111, 105, 107, 113, 106, 64, 79, 80, 90,
    ];
    Some(match key {
        0x41..=0x5A => LETTERS[(key - 0x41) as usize],
        0x30..=0x39 => DIGITS[(key - 0x30) as usize],
        0x70..=0x83 => FUNCTION[(key - 0x70) as usize],
        0x08 => 51,
        0x09 => 48,
        0x0D => 36,
        0x1B => 53,
        0x20 => 49,
        0x21 => 116,
        0x22 => 121,
        0x23 => 119,
        0x24 => 115,
        0x25 => 123,
        0x26 => 126,
        0x27 => 124,
        0x28 => 125,
        0x2D => 114,
        0x2E => 117,
        0x10 | 0xA0 => 56,
        0xA1 => 60,
        0x11 | 0xA2 => 59,
        0xA3 => 62,
        0x12 | 0xA4 => 58,
        0xA5 => 61,
        0x5B => 55,
        0x5C => 54,
        0x14 => 57,
        0xBA => 41,
        0xBB => 24,
        0xBC => 43,
        0xBD => 27,
        0xBE => 47,
        0xBF => 44,
        0xC0 => 50,
        0xDB => 33,
        0xDC => 42,
        0xDD => 30,
        0xDE => 39,
        0xE2 => 10,
        0x60 => 82,
        0x61 => 83,
        0x62 => 84,
        0x63 => 85,
        0x64 => 86,
        0x65 => 87,
        0x66 => 88,
        0x67 => 89,
        0x68 => 91,
        0x69 => 92,
        0x6A => 67,
        0x6B => 69,
        0x6D => 78,
        0x6E => 65,
        0x6F => 75,
        0x90 => 71,
        0xE01C => 76,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_keys_are_mapped_once_and_unknown_keys_are_rejected() {
        assert_eq!(virtual_key_to_macos(0x41), Some(0));
        assert_eq!(virtual_key_to_macos(0x5A), Some(6));
        assert_eq!(virtual_key_to_macos(0x5B), Some(55));
        assert_eq!(virtual_key_to_macos(0xA5), Some(61));
        assert_eq!(virtual_key_to_macos(0xE01C), Some(76));
        assert_eq!(virtual_key_to_macos(0), None);
        assert_eq!(virtual_key_to_macos(0xFFFF), None);
    }
    #[test]
    fn drag_events_preserve_held_button_semantics() {
        assert_eq!(
            drag_spec(None).unwrap().0 as u32,
            CGEventType::MouseMoved as u32
        );
        assert_eq!(
            drag_spec(Some(InputButton::Left)).unwrap().0 as u32,
            CGEventType::LeftMouseDragged as u32
        );
        assert_eq!(
            drag_spec(Some(InputButton::Right)).unwrap().0 as u32,
            CGEventType::RightMouseDragged as u32
        );
        assert_eq!(drag_spec(Some(InputButton::Other(2))).unwrap().2, 4);
        assert!(drag_spec(Some(InputButton::Other(99))).is_err());
    }
    #[test]
    fn releasing_one_shift_preserves_the_other_and_command_chords() {
        let mut injector = MacosInputInjector::new();
        injector.keys.extend([0xA0, 0xA1, 0x5B]);
        injector.keys.remove(&0xA0);
        assert!(injector.flags().contains(CGEventFlags::CGEventFlagShift));
        assert!(injector.flags().contains(CGEventFlags::CGEventFlagCommand));
        injector.keys.remove(&0xA1);
        assert!(!injector.flags().contains(CGEventFlags::CGEventFlagShift));
    }
}
