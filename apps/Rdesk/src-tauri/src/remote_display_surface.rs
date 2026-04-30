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
    #[cfg(any(windows, target_os = "macos"))]
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

    #[cfg(any(windows, target_os = "macos"))]
    pub fn render_target_handle(&self, label: &str) -> Option<isize> {
        self.surfaces
            .get(label)
            .map(NativeRenderSurface::render_target_handle)
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

    #[cfg(not(any(windows, target_os = "macos")))]
    pub fn render_target_handle(&self, _label: &str) -> Option<isize> {
        None
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    pub fn detach(
        &mut self,
        _label: &str,
        _window: Option<&WebviewWindow>,
    ) -> Result<bool, String> {
        Ok(false)
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    pub fn configure(
        &mut self,
        window: &WebviewWindow,
        rect: NativeSurfaceRect,
        enabled: bool,
        _visible: bool,
    ) -> Result<NativeRenderSurfaceSnapshot, String> {
        if enabled {
            return Err("DX11 native render surface is only available on Windows".to_string());
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
            CreateWindowExW, DefWindowProcW, LoadCursorW, RegisterClassW, SetWindowPos, CS_HREDRAW,
            CS_VREDRAW, HMENU, IDC_ARROW, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_SHOWWINDOW,
            WINDOW_EX_STYLE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
        };

        unsafe extern "system" fn wnd_proc(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
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
}

#[cfg(windows)]
impl Drop for NativeRenderSurface {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
        }
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
