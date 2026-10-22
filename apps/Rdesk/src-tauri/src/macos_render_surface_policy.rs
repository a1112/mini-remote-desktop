//! AppKit construction shared by the renderer and its native routing regression.
#![allow(deprecated, unexpected_cfgs)]

use cocoa::{
    appkit::{NSBackingStoreBuffered, NSView, NSWindow, NSWindowStyleMask},
    base::{id, nil, NO, YES},
    foundation::{NSPoint, NSRect},
};
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{Class, Object, Sel, BOOL},
    sel, sel_impl,
};
use std::sync::OnceLock;

// Registered Objective-C classes live for the lifetime of this process. Store only
// their addresses so the opaque objc::Class type is not given new Send/Sync traits.
static RENDER_VIEW_CLASS: OnceLock<Result<usize, &'static str>> = OnceLock::new();
static OVERLAY_WINDOW_CLASS: OnceLock<Result<usize, &'static str>> = OnceLock::new();

extern "C" fn decline_hit_test(_: &Object, _: Sel, _: NSPoint) -> id {
    nil
}

extern "C" fn decline_focus(_: &Object, _: Sel) -> BOOL {
    NO
}

unsafe fn require_main_thread() -> Result<(), String> {
    let main: BOOL = msg_send![class!(NSThread), isMainThread];
    if main == NO {
        return Err("macOS native render surfaces require the AppKit main thread".into());
    }
    Ok(())
}

unsafe fn render_view_class() -> Result<&'static Class, String> {
    let address = RENDER_VIEW_CLASS.get_or_init(|| {
        let mut declaration = ClassDecl::new("RdeskInputTransparentRenderViewV1", class!(NSView))
            .ok_or("register macOS input-transparent render view failed")?;
        declaration.add_method(
            sel!(hitTest:),
            decline_hit_test as extern "C" fn(&Object, Sel, NSPoint) -> id,
        );
        for selector in [
            sel!(acceptsFirstResponder),
            sel!(becomeFirstResponder),
            sel!(canBecomeKeyView),
        ] {
            declaration.add_method(
                selector,
                decline_focus as extern "C" fn(&Object, Sel) -> BOOL,
            );
        }
        Ok(declaration.register() as *const Class as usize)
    });
    address
        .as_ref()
        .map(|address| &*(*address as *const Class))
        .map_err(|error| (*error).to_string())
}

unsafe fn overlay_window_class() -> Result<&'static Class, String> {
    let address = OVERLAY_WINDOW_CLASS.get_or_init(|| {
        let mut declaration =
            ClassDecl::new("RdeskInputTransparentRenderWindowV1", class!(NSWindow))
                .ok_or("register macOS input-transparent overlay window failed")?;
        for selector in [sel!(canBecomeKeyWindow), sel!(canBecomeMainWindow)] {
            declaration.add_method(
                selector,
                decline_focus as extern "C" fn(&Object, Sel) -> BOOL,
            );
        }
        Ok(declaration.register() as *const Class as usize)
    });
    address
        .as_ref()
        .map(|address| &*(*address as *const Class))
        .map_err(|error| (*error).to_string())
}

/// Called on the AppKit main thread. The caller owns the returned allocation.
pub(crate) unsafe fn new_render_view(frame: NSRect) -> Result<id, String> {
    require_main_thread()?;
    let allocated: id = msg_send![render_view_class()?, alloc];
    let view = allocated.initWithFrame_(frame);
    if view == nil {
        return Err("create macOS native render NSView failed".into());
    }
    Ok(view)
}

/// Called on the AppKit main thread. The caller owns the returned allocation.
pub(crate) unsafe fn new_overlay_window(frame: NSRect) -> Result<id, String> {
    require_main_thread()?;
    let allocated: id = msg_send![overlay_window_class()?, alloc];
    let window = allocated.initWithContentRect_styleMask_backing_defer_(
        frame,
        NSWindowStyleMask::NSBorderlessWindowMask,
        NSBackingStoreBuffered,
        NO,
    );
    if window == nil {
        return Err("create macOS native render overlay NSWindow failed".into());
    }
    // Input belongs to the WKWebView window underneath this rendering overlay.
    let _: () = msg_send![window, setIgnoresMouseEvents: YES];
    Ok(window)
}
