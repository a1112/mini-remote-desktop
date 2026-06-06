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
    MouseHorizontalWheel {
        delta: i32,
    },
    Key {
        virtual_key: u16,
        pressed: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowsVirtualScreen {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
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
        InputEvent::MouseHorizontalWheel { delta } => {
            Ok(WindowsInputCommand::MouseHorizontalWheel { delta })
        }
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
        WindowsInputCommand::MouseMove { x, y } => send_input(mouse_move_input(x, y)),
        WindowsInputCommand::MouseButton { button, pressed } => {
            send_input(mouse_button_input(button, pressed))
        }
        WindowsInputCommand::MouseWheel { delta } => send_input(mouse_wheel_input(delta)),
        WindowsInputCommand::MouseHorizontalWheel { delta } => {
            send_input(mouse_horizontal_wheel_input(delta))
        }
        WindowsInputCommand::Key {
            virtual_key,
            pressed,
        } => send_input(key_input(virtual_key, pressed)),
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

fn mouse_move_input(x: i32, y: i32) -> ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    mouse_move_input_for_virtual_screen(x, y, current_virtual_screen())
}

fn current_virtual_screen() -> WindowsVirtualScreen {
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    WindowsVirtualScreen {
        left: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
        top: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
        width: unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) },
        height: unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) },
    }
}

fn mouse_move_input_for_virtual_screen(
    x: i32,
    y: i32,
    virtual_screen: WindowsVirtualScreen,
) -> ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };

    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: absolute_mouse_axis(x, virtual_screen.left, virtual_screen.width),
                dy: absolute_mouse_axis(y, virtual_screen.top, virtual_screen.height),
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn absolute_mouse_axis(coordinate: i32, origin: i32, extent: i32) -> i32 {
    if extent <= 0 {
        return 0;
    }
    let offset = i64::from(coordinate) - i64::from(origin);
    let value = ((offset * 65_536) + i64::from(extent / 2)) / i64::from(extent);
    value.clamp(0, 65_535) as i32
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

fn mouse_horizontal_wheel_input(
    delta: i32,
) -> ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_HWHEEL, MOUSEINPUT,
    };

    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: delta as u32,
                dwFlags: MOUSEEVENTF_HWHEEL,
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
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, VIRTUAL_KEY,
    };

    let scan_code = keyboard_scan_code(virtual_key);
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if scan_code != 0 {
        flags |= KEYEVENTF_SCANCODE;
    }
    if scan_code != 0 && is_extended_keyboard_scan_code(scan_code) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !pressed {
        flags |= KEYEVENTF_KEYUP;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(if scan_code == 0 { virtual_key } else { 0 }),
                wScan: scan_code,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn keyboard_scan_code(virtual_key: u16) -> u16 {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC_EX};

    unsafe { MapVirtualKeyW(u32::from(virtual_key), MAPVK_VK_TO_VSC_EX) as u16 }
}

fn is_extended_keyboard_scan_code(scan_code: u16) -> bool {
    matches!(scan_code >> 8, 0xE0 | 0xE1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InputButton, InputError, InputEvent, InputInjector, InputKey};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    static KEYBOARD_SMOKE_EVENTS: OnceLock<Mutex<Vec<KeyboardSmokeEvent>>> = OnceLock::new();

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
    fn windows_mapping_extended_mouse_buttons_preserves_button_and_pressed_state() {
        let cases = [
            (
                InputEvent::MouseButton {
                    button: InputButton::Other(1),
                    pressed: true,
                },
                WindowsInputCommand::MouseButton {
                    button: WindowsMouseButton::X1,
                    pressed: true,
                },
            ),
            (
                InputEvent::MouseButton {
                    button: InputButton::Other(2),
                    pressed: false,
                },
                WindowsInputCommand::MouseButton {
                    button: WindowsMouseButton::X2,
                    pressed: false,
                },
            ),
        ];

        for (event, expected) in cases {
            assert_eq!(
                map_windows_input(&event).expect("map extended button"),
                expected
            );
        }
    }

    #[test]
    fn windows_mapping_rejects_unsupported_extended_mouse_buttons() {
        assert_eq!(
            map_windows_input(&InputEvent::MouseButton {
                button: InputButton::Other(3),
                pressed: true,
            })
            .expect_err("unsupported extended button should be invalid"),
            InputError::InvalidEvent("unsupported mouse button 3".to_string())
        );
    }

    #[test]
    fn windows_mapping_wheel_delta_is_preserved() {
        assert_eq!(
            map_windows_input(&InputEvent::MouseWheel { delta: -240 }).expect("map wheel"),
            WindowsInputCommand::MouseWheel { delta: -240 }
        );
    }

    #[test]
    fn windows_mapping_horizontal_wheel_delta_is_preserved() {
        assert_eq!(
            map_windows_input(&InputEvent::MouseHorizontalWheel { delta: 120 })
                .expect("map horizontal wheel"),
            WindowsInputCommand::MouseHorizontalWheel { delta: 120 }
        );
    }

    #[test]
    fn windows_mouse_move_uses_sendinput_absolute_virtual_desktop_coordinates() {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
        };

        let input = mouse_move_input_for_virtual_screen(
            1920,
            0,
            WindowsVirtualScreen {
                left: -1920,
                top: 0,
                width: 3840,
                height: 2160,
            },
        );

        unsafe {
            assert_eq!(
                input.r#type,
                ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT_MOUSE
            );
            assert_eq!(
                input.Anonymous.mi.dwFlags,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK
            );
            assert_eq!(input.Anonymous.mi.dx, 65_535);
            assert_eq!(input.Anonymous.mi.dy, 0);
        }
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
    fn windows_key_input_populates_layout_scan_code() {
        use ::windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_SCANCODE;

        let input = key_input(0x41, true);

        unsafe {
            assert_eq!(
                input.r#type,
                ::windows::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD
            );
            assert_eq!(input.Anonymous.ki.wVk.0, 0);
            assert_ne!(input.Anonymous.ki.wScan, 0);
            assert_eq!(input.Anonymous.ki.dwFlags, KEYEVENTF_SCANCODE);
        }
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

    #[test]
    #[ignore = "manual smoke test: moves the local cursor and restores it"]
    fn windows_sendinput_mouse_move_smoke_moves_and_restores_cursor() {
        let start = current_cursor_position().expect("read starting cursor position");
        let target = (start.0.saturating_add(80), start.1.saturating_add(80));
        let mut injector = WindowsSendInputInjector::new();

        injector
            .inject(&InputEvent::MouseMove {
                x: target.0,
                y: target.1,
            })
            .expect("move cursor with SendInput");
        let after_move = current_cursor_position().expect("read cursor after SendInput move");
        let moved = wait_for_cursor_near(target, 4, Duration::from_millis(300))
            .expect("cursor reaches SendInput target");

        injector
            .inject(&InputEvent::MouseMove {
                x: start.0,
                y: start.1,
            })
            .expect("restore cursor with SendInput");
        let after_restore = current_cursor_position().expect("read cursor after SendInput restore");
        let restored = wait_for_cursor_near(start, 4, Duration::from_millis(300))
            .expect("cursor returns to starting position");
        force_cursor_position(start).expect("force exact cursor restore after smoke");

        eprintln!(
            "sendinput smoke virtual_screen={:?} start={:?} target={:?} after_move={:?} moved={:?} after_restore={:?} restored={:?}",
            current_virtual_screen(),
            start,
            target,
            after_move,
            moved,
            after_restore,
            restored
        );
        assert!(cursor_distance(after_move, start) > 10);
        assert!(moved.is_some());
        assert!(restored.is_some());
    }

    #[test]
    #[ignore = "manual smoke test: creates a focused window and sends a key through SendInput"]
    fn windows_sendinput_keyboard_smoke_sends_key_to_focused_test_window() {
        let mut window = KeyboardSmokeWindow::create().expect("create keyboard smoke window");
        window.focus();
        let mut injector = WindowsSendInputInjector::new();

        injector
            .inject(&InputEvent::Key {
                key: InputKey::VirtualKey(0x41),
                pressed: true,
            })
            .expect("send key down");
        injector
            .inject(&InputEvent::Key {
                key: InputKey::VirtualKey(0x41),
                pressed: false,
            })
            .expect("send key up");

        let events = window
            .wait_for_key_events(0x41, Duration::from_millis(500))
            .expect("wait for key events");
        eprintln!(
            "keyboard sendinput smoke focus={:?} events={:?}",
            window.focus_snapshot(),
            keyboard_smoke_events()
                .lock()
                .expect("read key smoke events")
        );
        assert!(events.key_down, "expected WM_KEYDOWN for virtual key 0x41");
        assert!(events.key_up, "expected WM_KEYUP for virtual key 0x41");
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum KeyboardSmokeEvent {
        KeyDown(u16),
        KeyUp(u16),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct KeyboardSmokeResult {
        key_down: bool,
        key_up: bool,
    }

    struct KeyboardSmokeWindow {
        hwnd: windows::Win32::Foundation::HWND,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct KeyboardSmokeFocusSnapshot {
        hwnd: isize,
        foreground: isize,
        focus: isize,
    }

    impl KeyboardSmokeWindow {
        fn create() -> windows::core::Result<Self> {
            use windows::core::PCWSTR;
            use windows::Win32::Foundation::HINSTANCE;
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;
            use windows::Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, RegisterClassW, ShowWindow, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
                SW_SHOW, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
            };

            keyboard_smoke_events()
                .lock()
                .expect("clear events")
                .clear();

            let class_name = wide_null(&format!(
                "MrdInputKeyboardSmoke{}{}",
                std::process::id(),
                current_time_millis()
            ));
            let title = wide_null("MRD input keyboard smoke");
            unsafe {
                let hmodule = GetModuleHandleW(None)?;
                let hinstance = HINSTANCE(hmodule.0);
                let window_class = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(keyboard_smoke_wnd_proc),
                    hInstance: hinstance,
                    lpszClassName: PCWSTR(class_name.as_ptr()),
                    ..Default::default()
                };
                if RegisterClassW(&window_class) == 0 {
                    return Err(windows::core::Error::from_thread());
                }

                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR(class_name.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    WS_OVERLAPPEDWINDOW,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    320,
                    160,
                    None,
                    None,
                    Some(hinstance),
                    None,
                )?;
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = windows::Win32::Graphics::Gdi::UpdateWindow(hwnd);
                pump_window_messages();

                Ok(Self { hwnd })
            }
        }

        fn focus(&mut self) {
            use windows::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
            use windows::Win32::UI::WindowsAndMessaging::{BringWindowToTop, SetForegroundWindow};

            unsafe {
                let _ = BringWindowToTop(self.hwnd);
                let _ = SetForegroundWindow(self.hwnd);
                let _ = SetActiveWindow(self.hwnd);
                let _ = SetFocus(Some(self.hwnd));
            }
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline {
                pump_window_messages();
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn wait_for_key_events(
            &mut self,
            virtual_key: u16,
            timeout: Duration,
        ) -> windows::core::Result<KeyboardSmokeResult> {
            let deadline = Instant::now() + timeout;
            loop {
                pump_window_messages();
                let result = keyboard_smoke_result(virtual_key);
                if result.key_down && result.key_up {
                    return Ok(result);
                }
                if Instant::now() >= deadline {
                    return Ok(result);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn focus_snapshot(&self) -> KeyboardSmokeFocusSnapshot {
            unsafe {
                KeyboardSmokeFocusSnapshot {
                    hwnd: self.hwnd.0 as isize,
                    foreground: windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0
                        as isize,
                    focus: windows::Win32::UI::Input::KeyboardAndMouse::GetFocus().0 as isize,
                }
            }
        }
    }

    impl Drop for KeyboardSmokeWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
            }
            pump_window_messages();
        }
    }

    unsafe extern "system" fn keyboard_smoke_wnd_proc(
        hwnd: windows::Win32::Foundation::HWND,
        message: u32,
        wparam: windows::Win32::Foundation::WPARAM,
        lparam: windows::Win32::Foundation::LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_KEYDOWN, WM_KEYUP};

        match message {
            WM_KEYDOWN => {
                keyboard_smoke_events()
                    .lock()
                    .expect("record key down")
                    .push(KeyboardSmokeEvent::KeyDown(wparam.0 as u16));
                windows::Win32::Foundation::LRESULT(0)
            }
            WM_KEYUP => {
                keyboard_smoke_events()
                    .lock()
                    .expect("record key up")
                    .push(KeyboardSmokeEvent::KeyUp(wparam.0 as u16));
                windows::Win32::Foundation::LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    fn keyboard_smoke_events() -> &'static Mutex<Vec<KeyboardSmokeEvent>> {
        KEYBOARD_SMOKE_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
    }

    fn keyboard_smoke_result(virtual_key: u16) -> KeyboardSmokeResult {
        let ime_process_key = windows::Win32::UI::Input::KeyboardAndMouse::VK_PROCESSKEY.0;
        let events = keyboard_smoke_events().lock().expect("read key events");
        // Some active IMEs report injected key-down messages as VK_PROCESSKEY,
        // while the key-up still carries the original virtual key.
        KeyboardSmokeResult {
            key_down: events.iter().any(|event| {
                matches!(
                    *event,
                    KeyboardSmokeEvent::KeyDown(key)
                        if key == virtual_key || key == ime_process_key
                )
            }),
            key_up: events.iter().any(|event| {
                matches!(
                    *event,
                    KeyboardSmokeEvent::KeyUp(key)
                        if key == virtual_key || key == ime_process_key
                )
            }),
        }
    }

    fn pump_window_messages() {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };

        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn current_time_millis() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default()
    }

    fn current_cursor_position() -> windows::core::Result<(i32, i32)> {
        let mut point = ::windows::Win32::Foundation::POINT::default();
        unsafe {
            ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point)?;
        }
        Ok((point.x, point.y))
    }

    fn wait_for_cursor_near(
        expected: (i32, i32),
        tolerance: i32,
        timeout: Duration,
    ) -> windows::core::Result<Option<(i32, i32)>> {
        let deadline = Instant::now() + timeout;
        loop {
            let current = current_cursor_position()?;
            if cursor_distance(current, expected) <= tolerance {
                return Ok(Some(current));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn cursor_distance(left: (i32, i32), right: (i32, i32)) -> i32 {
        left.0.abs_diff(right.0).max(left.1.abs_diff(right.1)) as i32
    }

    fn force_cursor_position(position: (i32, i32)) -> windows::core::Result<()> {
        unsafe { ::windows::Win32::UI::WindowsAndMessaging::SetCursorPos(position.0, position.1) }
    }
}
