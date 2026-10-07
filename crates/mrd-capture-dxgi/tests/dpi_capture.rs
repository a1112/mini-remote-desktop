#![cfg(windows)]

use mrd_capture_dxgi::{DxgiDesktopCapture, DxgiSharedTextureCapture};
use mrd_pipeline_core::FrameCapture;
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, GetThreadDpiAwarenessContext, SetThreadDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_UNAWARE,
};

struct RestoreContext(DPI_AWARENESS_CONTEXT);
impl Drop for RestoreContext {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

// Requires an interactive Windows desktop and a real desktop-duplication output.
// A scaled display reproduces the logical-coordinate regression directly.
#[test]
#[ignore]
fn shared_capture_uses_physical_pixels_and_restores_dpi_context() {
    let old = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE) };
    assert!(
        !old.0.is_null(),
        "set a DPI-unaware caller for this regression"
    );
    let _restore = RestoreContext(old);
    let mut mode = DEVMODEW::default();
    mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    assert!(
        unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut mode) }.as_bool()
    );
    let expected = (mode.dmPelsWidth as usize, mode.dmPelsHeight as usize);
    let mut capture = DxgiSharedTextureCapture::new_primary().expect("capture the primary output");
    assert_eq!(
        (capture.width(), capture.height()),
        expected,
        "capture dimensions must be physical pixels"
    );
    assert!(unsafe {
        AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_UNAWARE,
        )
    }
    .as_bool());
    let frame = capture
        .capture_frame()
        .expect("capture a real shared frame");
    assert_eq!((frame.width, frame.height), expected);
    assert!(unsafe {
        AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_UNAWARE,
        )
    }
    .as_bool());
    assert!(
        DxgiSharedTextureCapture::new_for_device_name("not-existing-dpi-regression-output")
            .is_err()
    );
    assert!(unsafe {
        AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_UNAWARE,
        )
    }
    .as_bool());
}

#[test]
#[ignore]
fn cpu_capture_uses_physical_pixels_and_restores_dpi_context() {
    let old = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE) };
    assert!(!old.0.is_null());
    let _restore = RestoreContext(old);
    let mut mode = DEVMODEW::default();
    mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    assert!(
        unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut mode) }.as_bool()
    );
    let expected = (mode.dmPelsWidth as usize, mode.dmPelsHeight as usize);
    let mut capture = DxgiDesktopCapture::new_primary().expect("capture CPU primary output");
    assert_eq!((capture.width(), capture.height()), expected);
    assert!(unsafe {
        AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_UNAWARE,
        )
    }
    .as_bool());
    let frame = capture
        .capture_frame()
        .expect("capture an actual CPU frame");
    assert_eq!((frame.width, frame.height), expected);
    assert!(unsafe {
        AreDpiAwarenessContextsEqual(
            GetThreadDpiAwarenessContext(),
            DPI_AWARENESS_CONTEXT_UNAWARE,
        )
    }
    .as_bool());
}
