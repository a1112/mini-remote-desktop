use crate::{InputButton, InputError, InputEvent, InputInjector, InputKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsMouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsInputCommand {
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: WindowsMouseButton,
        pressed: bool,
    },
    MouseWheel {
        delta: i32,
    },
    Key {
        virtual_key: u16,
        pressed: bool,
    },
}

#[derive(Debug, Default)]
pub struct WindowsSendInputInjector;

impl WindowsSendInputInjector {
    pub fn new() -> Self {
        Self
    }
}

impl InputInjector for WindowsSendInputInjector {
    fn is_available(&self) -> bool {
        true
    }

    fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
        send_windows_input(map_windows_input(event)?)
    }
}

pub fn map_windows_input(event: &InputEvent) -> Result<WindowsInputCommand, InputError> {
    match *event {
        InputEvent::MouseMove { x, y } => Ok(WindowsInputCommand::MouseMove { x, y }),
        InputEvent::MouseWheel { delta } => Ok(WindowsInputCommand::MouseWheel { delta }),
        InputEvent::MouseButton { button, pressed } => Ok(WindowsInputCommand::MouseButton {
            button: map_mouse_button(button)?,
            pressed,
        }),
        InputEvent::Key { key, pressed } => {
            let InputKey::VirtualKey(virtual_key) = key;
            if virtual_key == 0 {
                return Err(InputError::InvalidEvent(
                    "virtual key must be non-zero".to_string(),
                ));
            }
            Ok(WindowsInputCommand::Key {
                virtual_key,
                pressed,
            })
        }
    }
}

fn map_mouse_button(button: InputButton) -> Result<WindowsMouseButton, InputError> {
    match button {
        InputButton::Left => Ok(WindowsMouseButton::Left),
        InputButton::Right => Ok(WindowsMouseButton::Right),
        InputButton::Middle => Ok(WindowsMouseButton::Middle),
        InputButton::Other(1) => Ok(WindowsMouseButton::X1),
        InputButton::Other(2) => Ok(WindowsMouseButton::X2),
        InputButton::Other(other) => Err(InputError::InvalidEvent(format!(
            "unsupported mouse button {other}"
        ))),
    }
}

fn send_windows_input(command: WindowsInputCommand) -> Result<(), InputError> {
    match command {
        WindowsInputCommand::MouseMove { x, y } => {
            set_cursor_pos(x, y)?;
            Ok(())
        }
        WindowsInputCommand::MouseButton { button, pressed } => {
            send_input(mouse_button_input(button, pressed))
        }
        WindowsInputCommand::MouseWheel { delta } => send_input(mouse_wheel_input(delta)),
        WindowsInputCommand::Key {
            virtual_key,
            pressed,
        } => send_input(key_input(virtual_key, pressed)),
    }
}

fn set_cursor_pos(x: i32, y: i32) -> Result<(), InputError> {
    unsafe {
        ::windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y)
            .map_err(|err| InputError::Platform(err.to_string()))
    }
}

fn send_input(
    input: ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT,
) -> Result<(), InputError> {
    let inputs = [input];
    let sent = unsafe {
        ::windows::Win32::UI::Input::KeyboardAndMouse::SendInput(
            &inputs,
            std::mem::size_of::<::windows::Win32::UI::Input::KeyboardAndMouse::INPUT>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(InputError::Platform(
            ::windows::core::Error::from_thread().to_string(),
        ))
    }
}

fn mouse_button_input(
    button: WindowsMouseButton,
    pressed: bool,
) -> ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
    };

    let (flags, mouse_data) = match (button, pressed) {
        (WindowsMouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
        (WindowsMouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
        (WindowsMouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (WindowsMouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
        (WindowsMouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (WindowsMouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
        (WindowsMouseButton::X1, true) => (MOUSEEVENTF_XDOWN, 1),
        (WindowsMouseButton::X1, false) => (MOUSEEVENTF_XUP, 1),
        (WindowsMouseButton::X2, true) => (MOUSEEVENTF_XDOWN, 2),
        (WindowsMouseButton::X2, false) => (MOUSEEVENTF_XUP, 2),
    };

    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn mouse_wheel_input(delta: i32) -> ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT,
    };

    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: delta as u32,
                dwFlags: MOUSEEVENTF_WHEEL,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key_input(
    virtual_key: u16,
    pressed: bool,
) -> ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(virtual_key),
                wScan: 0,
                dwFlags: if pressed {
                    Default::default()
                } else {
                    KEYEVENTF_KEYUP
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InputButton, InputError, InputEvent, InputKey};

    #[test]
    fn windows_mapping_mouse_buttons_preserves_button_and_pressed_state() {
        let cases = [
            (
                InputEvent::MouseButton {
                    button: InputButton::Left,
                    pressed: true,
                },
                WindowsInputCommand::MouseButton {
                    button: WindowsMouseButton::Left,
                    pressed: true,
                },
            ),
            (
                InputEvent::MouseButton {
                    button: InputButton::Right,
                    pressed: false,
                },
                WindowsInputCommand::MouseButton {
                    button: WindowsMouseButton::Right,
                    pressed: false,
                },
            ),
            (
                InputEvent::MouseButton {
                    button: InputButton::Middle,
                    pressed: true,
                },
                WindowsInputCommand::MouseButton {
                    button: WindowsMouseButton::Middle,
                    pressed: true,
                },
            ),
        ];

        for (event, expected) in cases {
            assert_eq!(map_windows_input(&event).expect("map button"), expected);
        }
    }

    #[test]
    fn windows_mapping_wheel_delta_is_preserved() {
        assert_eq!(
            map_windows_input(&InputEvent::MouseWheel { delta: -240 }).expect("map wheel"),
            WindowsInputCommand::MouseWheel { delta: -240 }
        );
    }

    #[test]
    fn windows_mapping_virtual_key_preserves_code_and_pressed_state() {
        assert_eq!(
            map_windows_input(&InputEvent::Key {
                key: InputKey::VirtualKey(0x41),
                pressed: true,
            })
            .expect("map key"),
            WindowsInputCommand::Key {
                virtual_key: 0x41,
                pressed: true,
            }
        );
    }

    #[test]
    fn windows_mapping_rejects_empty_virtual_key() {
        assert_eq!(
            map_windows_input(&InputEvent::Key {
                key: InputKey::VirtualKey(0),
                pressed: true,
            })
            .expect_err("zero virtual key should be invalid"),
            InputError::InvalidEvent("virtual key must be non-zero".to_string())
        );
    }
}
