//! AppKit construction shared by the renderer and its native routing regression.
#![allow(deprecated, unexpected_cfgs)]

use cocoa::{
    appkit::{NSBackingStoreBuffered, NSView, NSWindow, NSWindowStyleMask},
    base::{id, nil, NO},
    foundation::NSRect,
};

/// Called on the AppKit main thread. The caller owns the returned allocation.
pub(crate) unsafe fn new_render_view(frame: NSRect) -> Result<id, String> {
    let view = NSView::alloc(nil).initWithFrame_(frame);
    if view == nil {
        return Err("create macOS native render NSView failed".into());
    }
    Ok(view)
}

/// Called on the AppKit main thread. The caller owns the returned allocation.
pub(crate) unsafe fn new_overlay_window(frame: NSRect) -> Result<id, String> {
    let window = NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
        frame,
        NSWindowStyleMask::NSBorderlessWindowMask,
        NSBackingStoreBuffered,
        NO,
    );
    if window == nil {
        return Err("create macOS native render overlay NSWindow failed".into());
    }
    Ok(window)
}
