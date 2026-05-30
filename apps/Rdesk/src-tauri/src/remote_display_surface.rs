#![allow(unexpected_cfgs)]

use serde::{Deserialize, Serialize};
use tauri::WebviewWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeSurfaceRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeRenderSurfaceSnapshot {
    pub label: String,
    pub backend: String,
    pub attached: bool,
    pub visible: bool,
    pub parent_hwnd: Option<String>,
    pub hwnd: Option<String>,
    pub rect: NativeSurfaceRect,
}

#[derive(Default)]
pub struct RemoteDisplaySurfaceManager {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    surfaces: std::collections::HashMap<String, NativeRenderSurface>,
}

impl RemoteDisplaySurfaceManager {
    #[cfg(windows)]
    pub fn configure(
        &mut self,
        window: &WebviewWindow,
        rect: NativeSurfaceRect,
        enabled: bool,
        visible: bool,
    ) -> Result<NativeRenderSurfaceSnapshot, String> {
        let label = window.label().to_string();

        if !enabled {
            self.surfaces.remove(&label);
            return Ok(NativeRenderSurfaceSnapshot {
                label,
                backend: "web".to_string(),
                attached: false,
                visible: false,
                parent_hwnd: None,
                hwnd: None,
                rect,
            });
        }

        let parent_hwnd = window
            .hwnd()
            .map_err(|error| format!("get remote display HWND failed: {error}"))?;
        let parent_hwnd = parent_hwnd.0 as isize;
        let rect = normalize_rect(rect);

        if let Some(surface) = self.surfaces.get_mut(&label) {
            surface.move_to(rect, visible)?;
            return Ok(surface.snapshot(label, rect));
        }

        let surface = NativeRenderSurface::create(parent_hwnd, rect, visible)?;
        let snapshot = surface.snapshot(label.clone(), rect);
        self.surfaces.insert(label, surface);
        Ok(snapshot)
    }

    #[cfg(target_os = "macos")]
    pub fn configure(
        &mut self,
        window: &WebviewWindow,
        rect: NativeSurfaceRect,
        enabled: bool,
        visible: bool,
    ) -> Result<NativeRenderSurfaceSnapshot, String> {
        let label = window.label().to_string();
        let rect = normalize_rect(rect);

        if !enabled {
            if let Some(surface) = self.surfaces.remove(&label) {
                surface.remove(window)?;
            }
            return Ok(NativeRenderSurfaceSnapshot {
                label,
                backend: "web".to_string(),
                attached: false,
                visible: false,
                parent_hwnd: None,
                hwnd: None,
                rect,
            });
        }

        let parent_ns_window = window
            .ns_window()
            .map_err(|error| format!("get remote display NSWindow failed: {error}"))?
            as isize;
        let webview_ns_view = window
            .ns_view()
            .map_err(|error| format!("get remote display WebView NSView failed: {error}"))?
            as isize;

        if let Some(surface) = self.surfaces.get_mut(&label) {
            surface.move_to(window, rect, visible)?;
            return Ok(surface.snapshot(label, rect));
        }

        let surface =
            NativeRenderSurface::create(window, parent_ns_window, webview_ns_view, rect, visible)?;
        let snapshot = surface.snapshot(label.clone(), rect);
        self.surfaces.insert(label, surface);
        Ok(snapshot)
    }

    #[cfg(target_os = "linux")]
    pub fn configure(
        &mut self,
        window: &WebviewWindow,
        rect: NativeSurfaceRect,
        enabled: bool,
        visible: bool,
    ) -> Result<NativeRenderSurfaceSnapshot, String> {
        let label = window.label().to_string();
        let rect = normalize_rect(rect);

        if !enabled {
            self.surfaces.remove(&label);
            return Ok(NativeRenderSurfaceSnapshot {
                label,
                backend: "web".to_string(),
                attached: false,
                visible: false,
                parent_hwnd: None,
                hwnd: None,
                rect,
            });
        }

        let parent_hwnd = linux_parent_x11_window(window)?;
        if let Some(surface) = self.surfaces.get_mut(&label) {
            surface.move_to(rect, visible)?;
            return Ok(surface.snapshot(label, rect));
        }

        let surface = NativeRenderSurface::create(parent_hwnd, rect, visible)?;
        let snapshot = surface.snapshot(label.clone(), rect);
        self.surfaces.insert(label, surface);
        Ok(snapshot)
    }

    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    pub fn render_target_handle(&self, label: &str) -> Option<isize> {
        self.surfaces
            .get(label)
            .map(NativeRenderSurface::render_target_handle)
    }

    #[cfg(windows)]
    pub fn set_control_session_id(
        &mut self,
        label: &str,
        session_id: Option<String>,
    ) -> Result<(), String> {
        let surface = self
            .surfaces
            .get_mut(label)
            .ok_or_else(|| format!("native render surface not found: {label}"))?;
        surface.set_control_session_id(session_id);
        Ok(())
    }

    #[cfg(windows)]
    pub fn detach(&mut self, label: &str, _window: Option<&WebviewWindow>) -> Result<bool, String> {
        Ok(self.surfaces.remove(label).is_some())
    }

    #[cfg(target_os = "macos")]
    pub fn detach(&mut self, label: &str, window: Option<&WebviewWindow>) -> Result<bool, String> {
        let Some(surface) = self.surfaces.remove(label) else {
            return Ok(false);
        };

        if let Some(window) = window {
            surface.remove(window)?;
        }

        Ok(true)
    }

    #[cfg(target_os = "linux")]
    pub fn detach(&mut self, label: &str, _window: Option<&WebviewWindow>) -> Result<bool, String> {
        Ok(self.surfaces.remove(label).is_some())
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    pub fn render_target_handle(&self, _label: &str) -> Option<isize> {
        None
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    pub fn detach(
        &mut self,
        _label: &str,
        _window: Option<&WebviewWindow>,
    ) -> Result<bool, String> {
        Ok(false)
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    pub fn configure(
        &mut self,
        window: &WebviewWindow,
        rect: NativeSurfaceRect,
        enabled: bool,
        _visible: bool,
    ) -> Result<NativeRenderSurfaceSnapshot, String> {
        if enabled {
            return Err(
                "embedded native render surface is only available on Windows, macOS, and Linux/X11"
                    .to_string(),
            );
        }

        Ok(NativeRenderSurfaceSnapshot {
            label: window.label().to_string(),
            backend: "web".to_string(),
            attached: false,
            visible: false,
            parent_hwnd: None,
            hwnd: None,
            rect: normalize_rect(rect),
        })
    }
}

fn normalize_rect(rect: NativeSurfaceRect) -> NativeSurfaceRect {
    NativeSurfaceRect {
        x: rect.x.max(0),
        y: rect.y.max(0),
        width: rect.width.max(1),
        height: rect.height.max(1),
    }
}

fn handle_hex(handle: isize) -> String {
    format!("0x{:X}", handle as usize)
}

#[cfg(windows)]
#[derive(Debug, Clone)]
pub struct NativeSurfaceControlInput {
    pub session_id: String,
    pub event: mrd_ipc::ControlInputEvent,
}

#[cfg(windows)]
static NATIVE_SURFACE_INPUT_FORWARDER: std::sync::OnceLock<
    std::sync::mpsc::Sender<NativeSurfaceControlInput>,
> = std::sync::OnceLock::new();

#[cfg(windows)]
pub fn install_control_input_forwarder(
    sender: std::sync::mpsc::Sender<NativeSurfaceControlInput>,
) -> bool {
    NATIVE_SURFACE_INPUT_FORWARDER.set(sender).is_ok()
}

#[cfg(windows)]
struct WindowsSurfaceInputContext {
    session_id: Option<String>,
}

#[cfg(windows)]
fn windows_mouse_coordinates_from_lparam(lparam: isize) -> (i32, i32) {
    let raw = lparam as u32;
    let x = i16::from_ne_bytes((raw as u16).to_ne_bytes()) as i32;
    let y = i16::from_ne_bytes(((raw >> 16) as u16).to_ne_bytes()) as i32;
    (x, y)
}

#[cfg(windows)]
fn windows_signed_high_word(value: usize) -> i32 {
    i16::from_ne_bytes(((value >> 16) as u16).to_ne_bytes()) as i32
}

#[cfg(windows)]
fn windows_surface_input_events_from_message(
    message: u32,
    wparam: usize,
    lparam: isize,
) -> Vec<mrd_ipc::ControlInputEvent> {
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_CANCELMODE, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
        WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
    };

    match message {
        WM_MOUSEMOVE => {
            let (x, y) = windows_mouse_coordinates_from_lparam(lparam);
            vec![mrd_ipc::ControlInputEvent::MouseMove { x, y }]
        }
        WM_MOUSEWHEEL => vec![mrd_ipc::ControlInputEvent::MouseWheel {
            delta: windows_signed_high_word(wparam),
        }],
        WM_LBUTTONDOWN => mouse_button_events(lparam, mrd_ipc::ControlInputButton::Left, true),
        WM_LBUTTONUP => mouse_button_events(lparam, mrd_ipc::ControlInputButton::Left, false),
        WM_RBUTTONDOWN => mouse_button_events(lparam, mrd_ipc::ControlInputButton::Right, true),
        WM_RBUTTONUP => mouse_button_events(lparam, mrd_ipc::ControlInputButton::Right, false),
        WM_MBUTTONDOWN => mouse_button_events(lparam, mrd_ipc::ControlInputButton::Middle, true),
        WM_MBUTTONUP => mouse_button_events(lparam, mrd_ipc::ControlInputButton::Middle, false),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            let button = match (wparam >> 16) & 0xffff {
                1 => mrd_ipc::ControlInputButton::X1,
                2 => mrd_ipc::ControlInputButton::X2,
                _ => return Vec::new(),
            };
            mouse_button_events(lparam, button, message == WM_XBUTTONDOWN)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => key_event(wparam, true).into_iter().collect(),
        WM_KEYUP | WM_SYSKEYUP => key_event(wparam, false).into_iter().collect(),
        WM_KILLFOCUS | WM_CANCELMODE => vec![mrd_ipc::ControlInputEvent::ReleaseAll],
        _ => Vec::new(),
    }
}

#[cfg(windows)]
fn mouse_button_events(
    lparam: isize,
    button: mrd_ipc::ControlInputButton,
    pressed: bool,
) -> Vec<mrd_ipc::ControlInputEvent> {
    let (x, y) = windows_mouse_coordinates_from_lparam(lparam);
    vec![
        mrd_ipc::ControlInputEvent::MouseMove { x, y },
        mouse_button_event(button, pressed),
    ]
}

#[cfg(windows)]
fn mouse_button_event(
    button: mrd_ipc::ControlInputButton,
    pressed: bool,
) -> mrd_ipc::ControlInputEvent {
    mrd_ipc::ControlInputEvent::MouseButton { button, pressed }
}

#[cfg(windows)]
fn key_event(wparam: usize, pressed: bool) -> Option<mrd_ipc::ControlInputEvent> {
    let code = u16::try_from(wparam).ok()?;
    Some(mrd_ipc::ControlInputEvent::Key {
        key: mrd_ipc::ControlInputKey::VirtualKey { code },
        pressed,
    })
}

#[cfg(windows)]
unsafe fn windows_surface_input_context_mut(
    hwnd: windows::Win32::Foundation::HWND,
) -> Option<&'static mut WindowsSurfaceInputContext> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWLP_USERDATA};
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowsSurfaceInputContext;
    ptr.as_mut()
}

#[cfg(windows)]
fn forward_windows_surface_input(
    hwnd: windows::Win32::Foundation::HWND,
    event: mrd_ipc::ControlInputEvent,
) {
    let Some(session_id) = (unsafe {
        windows_surface_input_context_mut(hwnd).and_then(|context| context.session_id.clone())
    }) else {
        return;
    };
    let Some(sender) = NATIVE_SURFACE_INPUT_FORWARDER.get() else {
        return;
    };
    let _ = sender.send(NativeSurfaceControlInput { session_id, event });
}

#[cfg(windows)]
struct NativeRenderSurface {
    parent_hwnd: isize,
    hwnd: windows::Win32::Foundation::HWND,
    visible: bool,
}

#[cfg(windows)]
impl NativeRenderSurface {
    fn create(parent_hwnd: isize, rect: NativeSurfaceRect, visible: bool) -> Result<Self, String> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, LoadCursorW, RegisterClassW, SetWindowLongPtrW,
            SetWindowPos, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HMENU, IDC_ARROW, SWP_HIDEWINDOW,
            SWP_NOACTIVATE, SWP_SHOWWINDOW, WINDOW_EX_STYLE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN,
            WS_CLIPSIBLINGS, WS_VISIBLE,
        };

        unsafe extern "system" fn wnd_proc(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            let events = windows_surface_input_events_from_message(message, wparam.0, lparam.0);
            if !events.is_empty() {
                for event in events {
                    forward_windows_surface_input(hwnd, event);
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }

        fn wide(value: &str) -> Vec<u16> {
            OsStr::new(value)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }

        let class_name = wide("RdeskRemoteDisplayNativeSurface");
        let title = wide("Rdesk Native Render Surface");
        let hmodule = unsafe { GetModuleHandleW(None) }
            .map_err(|error| format!("get module handle failed: {error}"))?;
        let hinstance = HINSTANCE(hmodule.0);
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }
            .map_err(|error| format!("load cursor failed: {error}"))?;

        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            hCursor: cursor,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&window_class);
        }

        let style = if visible {
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN
        } else {
            WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN
        };

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                style,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                HWND(parent_hwnd),
                HMENU(0),
                hinstance,
                None,
            )
        };
        if hwnd.0 == 0 {
            return Err("create native render surface failed".to_string());
        }

        let context = Box::new(WindowsSurfaceInputContext { session_id: None });
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(context) as isize);
        }

        unsafe {
            SetWindowPos(
                hwnd,
                HWND(0),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE
                    | if visible {
                        SWP_SHOWWINDOW
                    } else {
                        SWP_HIDEWINDOW
                    },
            )
            .map_err(|error| format!("position native render surface failed: {error}"))?;
        }

        Ok(Self {
            parent_hwnd,
            hwnd,
            visible,
        })
    }

    fn move_to(&mut self, rect: NativeSurfaceRect, visible: bool) -> Result<(), String> {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_SHOWWINDOW,
        };

        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND(0),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE
                    | if visible {
                        SWP_SHOWWINDOW
                    } else {
                        SWP_HIDEWINDOW
                    },
            )
            .map_err(|error| format!("position native render surface failed: {error}"))?;
        }
        self.visible = visible;
        Ok(())
    }

    fn snapshot(&self, label: String, rect: NativeSurfaceRect) -> NativeRenderSurfaceSnapshot {
        NativeRenderSurfaceSnapshot {
            label,
            backend: "d3d11".to_string(),
            attached: true,
            visible: self.visible,
            parent_hwnd: Some(handle_hex(self.parent_hwnd)),
            hwnd: Some(handle_hex(self.hwnd.0)),
            rect,
        }
    }

    fn render_target_handle(&self) -> isize {
        self.hwnd.0
    }

    fn set_control_session_id(&mut self, session_id: Option<String>) {
        unsafe {
            if let Some(context) = windows_surface_input_context_mut(self.hwnd) {
                context.session_id = session_id;
            }
        }
    }
}

#[cfg(windows)]
impl Drop for NativeRenderSurface {
    fn drop(&mut self) {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_USERDATA};
            let ptr =
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0) as *mut WindowsSurfaceInputContext;
            if !ptr.is_null() {
                drop(Box::from_raw(ptr));
            }
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
        }
    }
}

#[cfg(target_os = "linux")]
struct NativeRenderSurface {
    parent_hwnd: isize,
    display: *mut x11::xlib::Display,
    window: x11::xlib::Window,
    visible: bool,
}

#[cfg(target_os = "linux")]
unsafe impl Send for NativeRenderSurface {}

#[cfg(target_os = "linux")]
impl NativeRenderSurface {
    fn create(parent_hwnd: isize, rect: NativeSurfaceRect, visible: bool) -> Result<Self, String> {
        use std::ptr;
        use x11::xlib;

        if parent_hwnd == 0 {
            return Err("remote display Linux parent X11 window is null".to_string());
        }

        unsafe {
            init_x11_threads();
            let display = (xlib::XOpenDisplay)(ptr::null());
            if display.is_null() {
                return Err(
                    "open X11 display failed; embedded Linux native render requires X11/XWayland"
                        .to_string(),
                );
            }

            let screen = (xlib::XDefaultScreen)(display);
            let black = (xlib::XBlackPixel)(display, screen);
            let window = (xlib::XCreateSimpleWindow)(
                display,
                parent_hwnd as xlib::Window,
                rect.x,
                rect.y,
                rect.width as u32,
                rect.height as u32,
                0,
                black,
                black,
            );
            if window == 0 {
                (xlib::XCloseDisplay)(display);
                return Err("create Linux native render child window failed".to_string());
            }

            (xlib::XSelectInput)(
                display,
                window,
                xlib::ExposureMask | xlib::StructureNotifyMask,
            );
            if visible {
                (xlib::XMapRaised)(display, window);
                (xlib::XRaiseWindow)(display, window);
            }
            (xlib::XFlush)(display);

            Ok(Self {
                parent_hwnd,
                display,
                window,
                visible,
            })
        }
    }

    fn move_to(&mut self, rect: NativeSurfaceRect, visible: bool) -> Result<(), String> {
        use x11::xlib;

        unsafe {
            if self.display.is_null() || self.window == 0 {
                return Err("Linux native render surface is detached".to_string());
            }
            (xlib::XMoveResizeWindow)(
                self.display,
                self.window,
                rect.x,
                rect.y,
                rect.width as u32,
                rect.height as u32,
            );
            if visible {
                (xlib::XMapRaised)(self.display, self.window);
                (xlib::XRaiseWindow)(self.display, self.window);
            } else {
                (xlib::XUnmapWindow)(self.display, self.window);
            }
            (xlib::XFlush)(self.display);
        }

        self.visible = visible;
        Ok(())
    }

    fn snapshot(&self, label: String, rect: NativeSurfaceRect) -> NativeRenderSurfaceSnapshot {
        NativeRenderSurfaceSnapshot {
            label,
            backend: "linux".to_string(),
            attached: true,
            visible: self.visible,
            parent_hwnd: Some(handle_hex(self.parent_hwnd)),
            hwnd: Some(handle_hex(self.window as isize)),
            rect,
        }
    }

    fn render_target_handle(&self) -> isize {
        self.window as isize
    }
}

#[cfg(target_os = "linux")]
impl Drop for NativeRenderSurface {
    fn drop(&mut self) {
        use x11::xlib;

        unsafe {
            if !self.display.is_null() {
                if self.window != 0 {
                    (xlib::XDestroyWindow)(self.display, self.window);
                    (xlib::XFlush)(self.display);
                }
                (xlib::XCloseDisplay)(self.display);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_parent_x11_window(window: &WebviewWindow) -> Result<isize, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window
        .window_handle()
        .map_err(|error| format!("get remote display native window handle failed: {error}"))?;

    match handle.as_raw() {
        RawWindowHandle::Xlib(handle) => {
            if handle.window == 0 {
                Err("remote display Linux Xlib window handle is null".to_string())
            } else {
                Ok(handle.window as isize)
            }
        }
        RawWindowHandle::Xcb(handle) => Ok(handle.window.get() as isize),
        RawWindowHandle::Wayland(_) => Err(
            "embedded Linux native render currently requires X11/XWayland; switch the session to X11 or use Web View on Wayland"
                .to_string(),
        ),
        other => Err(format!(
            "embedded Linux native render requires an X11 window handle, got {other:?}"
        )),
    }
}

#[cfg(target_os = "linux")]
fn init_x11_threads() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| unsafe {
        let _ = x11::xlib::XInitThreads();
    });
}

#[cfg(all(test, windows))]
mod remote_display_surface_input_tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::time::Duration;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SendMessageW, CS_HREDRAW,
        CS_VREDRAW, HMENU, WINDOW_EX_STYLE, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WNDCLASSW, WS_OVERLAPPED,
    };

    fn lparam(x: i16, y: i16) -> isize {
        ((u16::from_ne_bytes(y.to_ne_bytes()) as u32) << 16
            | u16::from_ne_bytes(x.to_ne_bytes()) as u32) as isize
    }

    fn wide(value: &str) -> Vec<u16> {
        OsStr::new(value)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    struct TestParentWindow(HWND);

    impl TestParentWindow {
        fn create() -> Self {
            unsafe extern "system" fn wnd_proc(
                hwnd: HWND,
                message: u32,
                wparam: WPARAM,
                lparam: LPARAM,
            ) -> windows::Win32::Foundation::LRESULT {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }

            let class_name = wide("RdeskRemoteDisplayNativeSurfaceTestParent");
            let title = wide("Rdesk Native Surface Test Parent");
            let hmodule = unsafe { GetModuleHandleW(None) }.expect("get module handle");
            let hinstance = HINSTANCE(hmodule.0);
            let window_class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance,
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            unsafe {
                RegisterClassW(&window_class);
            }

            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR(class_name.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    WS_OVERLAPPED,
                    0,
                    0,
                    128,
                    128,
                    HWND(0),
                    HMENU(0),
                    hinstance,
                    None,
                )
            };
            assert_ne!(hwnd.0, 0, "create parent window");
            Self(hwnd)
        }
    }

    impl Drop for TestParentWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    #[test]
    fn remote_display_surface_input_maps_signed_mouse_coordinates() {
        assert_eq!(
            windows_mouse_coordinates_from_lparam(lparam(-2, 300)),
            (-2, 300)
        );
        assert_eq!(
            windows_surface_input_events_from_message(WM_MOUSEMOVE, 0, lparam(640, 360)),
            vec![mrd_ipc::ControlInputEvent::MouseMove { x: 640, y: 360 }]
        );
    }

    #[test]
    fn remote_display_surface_input_maps_button_and_wheel_messages() {
        assert_eq!(
            windows_surface_input_events_from_message(WM_LBUTTONDOWN, 0, lparam(0, 0)),
            vec![
                mrd_ipc::ControlInputEvent::MouseMove { x: 0, y: 0 },
                mrd_ipc::ControlInputEvent::MouseButton {
                    button: mrd_ipc::ControlInputButton::Left,
                    pressed: true,
                },
            ]
        );
        assert_eq!(
            windows_surface_input_events_from_message(WM_LBUTTONUP, 0, lparam(0, 0)),
            vec![
                mrd_ipc::ControlInputEvent::MouseMove { x: 0, y: 0 },
                mrd_ipc::ControlInputEvent::MouseButton {
                    button: mrd_ipc::ControlInputButton::Left,
                    pressed: false,
                },
            ]
        );
        assert_eq!(
            windows_surface_input_events_from_message(WM_MOUSEWHEEL, (120_u16 as usize) << 16, 0),
            vec![mrd_ipc::ControlInputEvent::MouseWheel { delta: 120 }]
        );
    }

    #[test]
    fn remote_display_surface_input_moves_cursor_before_button_press() {
        assert_eq!(
            windows_surface_input_events_from_message(WM_LBUTTONDOWN, 0, lparam(640, 360)),
            vec![
                mrd_ipc::ControlInputEvent::MouseMove { x: 640, y: 360 },
                mrd_ipc::ControlInputEvent::MouseButton {
                    button: mrd_ipc::ControlInputButton::Left,
                    pressed: true,
                },
            ]
        );
    }

    #[test]
    fn remote_display_surface_input_maps_key_and_focus_loss_messages() {
        assert_eq!(
            windows_surface_input_events_from_message(WM_KEYDOWN, 0x41, 0),
            vec![mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: true,
            }]
        );
        assert_eq!(
            windows_surface_input_events_from_message(WM_KEYUP, 0x41, 0),
            vec![mrd_ipc::ControlInputEvent::Key {
                key: mrd_ipc::ControlInputKey::VirtualKey { code: 0x41 },
                pressed: false,
            }]
        );
        assert_eq!(
            windows_surface_input_events_from_message(WM_KILLFOCUS, 0, 0),
            vec![mrd_ipc::ControlInputEvent::ReleaseAll]
        );
    }

    #[test]
    fn remote_display_surface_input_forwards_wndproc_events_with_session_id() {
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(
            install_control_input_forwarder(sender),
            "native surface input forwarder should only be installed once in tests"
        );

        let parent = TestParentWindow::create();
        let parent_hwnd = parent.0;
        let mut surface = NativeRenderSurface::create(
            parent_hwnd.0,
            NativeSurfaceRect {
                x: 0,
                y: 0,
                width: 128,
                height: 128,
            },
            false,
        )
        .expect("create native render surface");
        surface.set_control_session_id(Some("native-forward-session".to_string()));

        unsafe {
            SendMessageW(
                surface.hwnd,
                WM_MOUSEMOVE,
                WPARAM(0),
                LPARAM(lparam(42, 24)),
            );
        }

        let input = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("forwarded native surface control input");
        assert_eq!(input.session_id, "native-forward-session");
        assert_eq!(
            input.event,
            mrd_ipc::ControlInputEvent::MouseMove { x: 42, y: 24 }
        );
    }
}

#[cfg(target_os = "macos")]
struct NativeRenderSurface {
    parent_ns_window: isize,
    webview_ns_view: isize,
    ns_view: isize,
    visible: bool,
}

#[cfg(target_os = "macos")]
impl NativeRenderSurface {
    fn create(
        window: &WebviewWindow,
        parent_ns_window: isize,
        webview_ns_view: isize,
        rect: NativeSurfaceRect,
        visible: bool,
    ) -> Result<Self, String> {
        let ns_view = run_on_main_thread(window, move || unsafe {
            create_macos_native_surface(parent_ns_window, webview_ns_view, rect, visible)
        })?;

        Ok(Self {
            parent_ns_window,
            webview_ns_view,
            ns_view,
            visible,
        })
    }

    fn move_to(
        &mut self,
        window: &WebviewWindow,
        rect: NativeSurfaceRect,
        visible: bool,
    ) -> Result<(), String> {
        let ns_view = self.ns_view;
        let parent_ns_window = self.parent_ns_window;
        let webview_ns_view = self.webview_ns_view;
        run_on_main_thread(window, move || unsafe {
            move_macos_native_surface(parent_ns_window, webview_ns_view, ns_view, rect, visible)
        })?;
        self.visible = visible;
        Ok(())
    }

    fn remove(self, window: &WebviewWindow) -> Result<(), String> {
        let ns_view = self.ns_view;
        run_on_main_thread(window, move || unsafe {
            remove_macos_native_surface(ns_view);
            Ok(())
        })
    }

    fn snapshot(&self, label: String, rect: NativeSurfaceRect) -> NativeRenderSurfaceSnapshot {
        NativeRenderSurfaceSnapshot {
            label,
            backend: "macos".to_string(),
            attached: true,
            visible: self.visible,
            parent_hwnd: Some(handle_hex(self.parent_ns_window)),
            hwnd: Some(handle_hex(self.ns_view)),
            rect,
        }
    }

    fn render_target_handle(&self) -> isize {
        self.ns_view
    }
}

#[cfg(target_os = "macos")]
fn run_on_main_thread<T, F>(window: &WebviewWindow, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::channel();
    window
        .run_on_main_thread(move || {
            let _ = sender.send(f());
        })
        .map_err(|error| format!("schedule macOS native surface update failed: {error}"))?;

    receiver
        .recv()
        .map_err(|error| format!("macOS native surface update failed: {error}"))?
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
unsafe fn create_macos_native_surface(
    parent_ns_window: isize,
    webview_ns_view: isize,
    rect: NativeSurfaceRect,
    visible: bool,
) -> Result<isize, String> {
    use cocoa::{
        appkit::{NSView, NSWindowOrderingMode},
        base::{id, nil, NO, YES},
    };
    use objc::{msg_send, sel, sel_impl};

    let ns_window = parent_ns_window as id;
    let webview = webview_ns_view as id;
    if ns_window == nil || webview == nil {
        return Err("remote display macOS parent pointer is null".to_string());
    }

    let content_view: id = msg_send![ns_window, contentView];
    if content_view == nil {
        return Err("remote display NSWindow has no contentView".to_string());
    }

    let frame = rect_to_content_view_frame(content_view, webview, rect);
    let view: id = NSView::alloc(nil).initWithFrame_(frame);
    if view == nil {
        return Err("create macOS native render NSView failed".to_string());
    }

    view.setWantsLayer(YES);
    view.setAutoresizingMask_(0);
    let _: () = msg_send![view, setHidden: if visible { NO } else { YES }];
    let _: () = msg_send![view, setPostsFrameChangedNotifications: YES];
    let _: () = msg_send![
        content_view,
        addSubview: view
        positioned: NSWindowOrderingMode::NSWindowAbove
        relativeTo: nil
    ];

    // Keep one retain count owned by the surface manager until remove().
    let _: id = msg_send![view, retain];
    Ok(view as isize)
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
unsafe fn move_macos_native_surface(
    parent_ns_window: isize,
    webview_ns_view: isize,
    ns_view: isize,
    rect: NativeSurfaceRect,
    visible: bool,
) -> Result<(), String> {
    use cocoa::{
        appkit::{NSView, NSWindowOrderingMode},
        base::{id, nil, NO, YES},
    };
    use objc::{msg_send, sel, sel_impl};

    let ns_window = parent_ns_window as id;
    let webview = webview_ns_view as id;
    let view = ns_view as id;
    if ns_window == nil || webview == nil || view == nil {
        return Err("macOS native surface pointer is null".to_string());
    }

    let content_view: id = msg_send![ns_window, contentView];
    if content_view == nil {
        return Err("remote display NSWindow has no contentView".to_string());
    }

    let frame = rect_to_content_view_frame(content_view, webview, rect);
    view.setFrameOrigin(frame.origin);
    view.setFrameSize(frame.size);
    let _: () = msg_send![view, setHidden: if visible { NO } else { YES }];
    sync_macos_surface_layer_frame(view);
    let _: () = msg_send![
        content_view,
        addSubview: view
        positioned: NSWindowOrderingMode::NSWindowAbove
        relativeTo: nil
    ];
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(deprecated, unexpected_cfgs)]
unsafe fn remove_macos_native_surface(ns_view: isize) {
    use cocoa::{appkit::NSView, base::id};
    use objc::{msg_send, sel, sel_impl};

    let view = ns_view as id;
    if !view.is_null() {
        view.removeFromSuperview();
        let _: () = msg_send![view, release];
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn rect_to_content_view_frame(
    content_view: cocoa::base::id,
    webview: cocoa::base::id,
    rect: NativeSurfaceRect,
) -> cocoa::foundation::NSRect {
    use cocoa::{appkit::NSView, foundation::NSRect};
    use objc::{msg_send, sel, sel_impl};

    let webview_bounds: NSRect = NSView::bounds(webview);
    let webview_frame = rect_to_bottom_left_frame(rect, webview_bounds.size.height);
    msg_send![content_view, convertRect: webview_frame fromView: webview]
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn rect_to_bottom_left_frame(
    rect: NativeSurfaceRect,
    parent_height: f64,
) -> cocoa::foundation::NSRect {
    use cocoa::foundation::{NSPoint, NSRect, NSSize};

    let y = (parent_height - rect.y as f64 - rect.height as f64).max(0.0);
    NSRect::new(
        NSPoint::new(rect.x as f64, y),
        NSSize::new(rect.width as f64, rect.height as f64),
    )
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn sync_macos_surface_layer_frame(view: cocoa::base::id) {
    use cocoa::appkit::NSView;
    use objc::{msg_send, runtime::Object, sel, sel_impl};

    let layer: *mut Object = msg_send![view, layer];
    if layer.is_null() {
        return;
    }

    let bounds = NSView::bounds(view);
    let window: *mut Object = msg_send![view, window];
    let contents_scale = if window.is_null() {
        1.0
    } else {
        msg_send![window, backingScaleFactor]
    };
    let _: () = msg_send![layer, setFrame: bounds];
    let _: () = msg_send![layer, setContentsScale: contents_scale];
}

#[cfg(test)]
mod tests {
    use super::{normalize_rect, NativeSurfaceRect};

    #[test]
    fn native_surface_rect_is_clamped_to_visible_size() {
        let rect = normalize_rect(NativeSurfaceRect {
            x: -10,
            y: -20,
            width: 0,
            height: -1,
        });

        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.width, 1);
        assert_eq!(rect.height, 1);
    }

    #[cfg(target_os = "macos")]
    #[allow(deprecated)]
    #[test]
    fn macos_rect_is_converted_from_web_top_left_to_appkit_bottom_left() {
        let frame = super::rect_to_bottom_left_frame(
            NativeSurfaceRect {
                x: 20,
                y: 56,
                width: 800,
                height: 400,
            },
            900.0,
        );

        assert_eq!(frame.origin.x, 20.0);
        assert_eq!(frame.origin.y, 444.0);
        assert_eq!(frame.size.width, 800.0);
        assert_eq!(frame.size.height, 400.0);
    }
}
