use thiserror::Error;

#[cfg(windows)]
pub mod windows;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputButton {
    Left,
    Right,
    Middle,
    Other(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputKey {
    VirtualKey(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputEvent {
    MouseMove { x: i32, y: i32 },
    MouseButton { button: InputButton, pressed: bool },
    MouseWheel { delta: i32 },
    Key { key: InputKey, pressed: bool },
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InputError {
    #[error("input injector unavailable: {0}")]
    Unavailable(String),
    #[error("invalid input event: {0}")]
    InvalidEvent(String),
    #[error("platform input injection failed: {0}")]
    Platform(String),
}

pub trait InputInjector {
    fn is_available(&self) -> bool;
    fn inject(&mut self, event: &InputEvent) -> Result<(), InputError>;
}

#[derive(Debug, Clone)]
pub struct RecordingInputInjector {
    available: bool,
    unavailable_reason: String,
    recorded: Vec<InputEvent>,
}

impl RecordingInputInjector {
    pub fn available() -> Self {
        Self {
            available: true,
            unavailable_reason: String::new(),
            recorded: Vec::new(),
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            unavailable_reason: reason.into(),
            recorded: Vec::new(),
        }
    }

    pub fn recorded(&self) -> &[InputEvent] {
        &self.recorded
    }
}

impl InputInjector for RecordingInputInjector {
    fn is_available(&self) -> bool {
        self.available
    }

    fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
        if !self.available {
            return Err(InputError::Unavailable(self.unavailable_reason.clone()));
        }
        self.recorded.push(*event);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct UnsupportedInputInjector {
    reason: String,
}

impl UnsupportedInputInjector {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl InputInjector for UnsupportedInputInjector {
    fn is_available(&self) -> bool {
        false
    }

    fn inject(&mut self, _event: &InputEvent) -> Result<(), InputError> {
        Err(InputError::Unavailable(self.reason.clone()))
    }
}

#[derive(Debug, Clone)]
pub struct TrackedInputInjector<I> {
    inner: I,
    active_buttons: Vec<InputButton>,
    active_keys: Vec<InputKey>,
}

impl<I> TrackedInputInjector<I> {
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            active_buttons: Vec::new(),
            active_keys: Vec::new(),
        }
    }

    pub fn inner(&self) -> &I {
        &self.inner
    }

    pub fn into_inner(self) -> I {
        self.inner
    }
}

impl<I: InputInjector> TrackedInputInjector<I> {
    pub fn release_all(&mut self) -> Result<Vec<InputEvent>, InputError> {
        let mut released = Vec::with_capacity(self.active_buttons.len() + self.active_keys.len());

        for button in self.active_buttons.clone() {
            let event = InputEvent::MouseButton {
                button,
                pressed: false,
            };
            self.inner.inject(&event)?;
            released.push(event);
        }
        self.active_buttons.clear();

        for key in self.active_keys.clone() {
            let event = InputEvent::Key {
                key,
                pressed: false,
            };
            self.inner.inject(&event)?;
            released.push(event);
        }
        self.active_keys.clear();

        Ok(released)
    }

    fn update_pressed_state(&mut self, event: &InputEvent) {
        match *event {
            InputEvent::MouseButton { button, pressed } => {
                if pressed {
                    push_unique(&mut self.active_buttons, button);
                } else {
                    self.active_buttons.retain(|active| *active != button);
                }
            }
            InputEvent::Key { key, pressed } => {
                if pressed {
                    push_unique(&mut self.active_keys, key);
                } else {
                    self.active_keys.retain(|active| *active != key);
                }
            }
            InputEvent::MouseMove { .. } | InputEvent::MouseWheel { .. } => {}
        }
    }
}

impl<I: InputInjector> InputInjector for TrackedInputInjector<I> {
    fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
        self.inner.inject(event)?;
        self.update_pressed_state(event);
        Ok(())
    }
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, value: T) {
    if !items.contains(&value) {
        items.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_injector_records_all_input_event_kinds() {
        let mut injector = RecordingInputInjector::available();
        let events = [
            InputEvent::MouseMove { x: 10, y: 20 },
            InputEvent::MouseWheel { delta: -120 },
            InputEvent::MouseButton {
                button: InputButton::Left,
                pressed: true,
            },
            InputEvent::Key {
                key: InputKey::VirtualKey(0x41),
                pressed: true,
            },
        ];

        for event in events {
            injector.inject(&event).expect("recording injection");
        }

        assert_eq!(injector.recorded(), events.as_slice());
    }

    #[test]
    fn tracked_injector_releases_active_keys_and_buttons() {
        let recorder = RecordingInputInjector::available();
        let mut injector = TrackedInputInjector::new(recorder);

        injector
            .inject(&InputEvent::Key {
                key: InputKey::VirtualKey(0x41),
                pressed: true,
            })
            .expect("key down");
        injector
            .inject(&InputEvent::MouseButton {
                button: InputButton::Left,
                pressed: true,
            })
            .expect("button down");

        let released = injector.release_all().expect("release all");

        assert_eq!(
            released,
            vec![
                InputEvent::MouseButton {
                    button: InputButton::Left,
                    pressed: false,
                },
                InputEvent::Key {
                    key: InputKey::VirtualKey(0x41),
                    pressed: false,
                },
            ]
        );
        assert_eq!(
            injector.inner().recorded(),
            &[
                InputEvent::Key {
                    key: InputKey::VirtualKey(0x41),
                    pressed: true,
                },
                InputEvent::MouseButton {
                    button: InputButton::Left,
                    pressed: true,
                },
                InputEvent::MouseButton {
                    button: InputButton::Left,
                    pressed: false,
                },
                InputEvent::Key {
                    key: InputKey::VirtualKey(0x41),
                    pressed: false,
                },
            ]
        );
    }

    #[test]
    fn unsupported_injector_reports_unavailable_and_rejects_input() {
        let mut injector = UnsupportedInputInjector::new("not implemented");

        assert!(!injector.is_available());
        assert_eq!(
            injector
                .inject(&InputEvent::MouseMove { x: 1, y: 2 })
                .expect_err("unsupported should reject"),
            InputError::Unavailable("not implemented".to_string())
        );
    }
}
