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
    pub parent_hwnd: Option<isize>,
    pub hwnd: Option<isize>,
    pub rect: NativeSurfaceRect,
}

#[derive(Default)]
pub struct RemoteDisplaySurfaceManager {
    #[cfg(windows)]
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

    #[cfg(not(windows))]
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
            parent_hwnd: Some(self.parent_hwnd),
            hwnd: Some(self.hwnd.0),
            rect,
        }
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
}
