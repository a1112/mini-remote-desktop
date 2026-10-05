#![allow(deprecated, unexpected_cfgs)]

#[cfg(target_os = "macos")]
#[path = "../../../apps/Rdesk/src-tauri/src/macos_render_surface_policy.rs"]
mod policy;

#[cfg(target_os = "macos")]
mod proof {
    use cocoa::{
        appkit::{
            NSBackingStoreBuffered, NSView, NSWindow, NSWindowOrderingMode, NSWindowStyleMask,
        },
        base::{id, nil, NO, YES},
        foundation::{NSAutoreleasePool, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString},
    };
    use objc::{
        class,
        declare::ClassDecl,
        msg_send,
        runtime::{Class, Object, Sel, BOOL},
        sel, sel_impl,
    };
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

    static MOUSE_DOWN: AtomicUsize = AtomicUsize::new(0);
    static KEY_DOWN: AtomicUsize = AtomicUsize::new(0);

    struct Owned(id);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg_send![self.0, release];
            }
        }
    }

    extern "C" fn accepts_first_responder(_: &Object, _: Sel) -> BOOL {
        YES
    }
    extern "C" fn accepts_first_mouse(_: &Object, _: Sel, _: id) -> BOOL {
        YES
    }
    extern "C" fn mouse_down(view: &Object, _: Sel, _: id) {
        MOUSE_DOWN.fetch_add(1, Ordering::SeqCst);
        unsafe {
            let window: id = msg_send![view, window];
            let _: BOOL = msg_send![window, makeFirstResponder: view];
        }
    }
    extern "C" fn key_down(_: &Object, _: Sel, _: id) {
        KEY_DOWN.fetch_add(1, Ordering::SeqCst);
    }

    unsafe fn receiver_class() -> &'static Class {
        let mut declaration =
            ClassDecl::new("RdeskNativeSurfaceProofReceiverV1", class!(NSView)).unwrap();
        declaration.add_method(
            sel!(acceptsFirstResponder),
            accepts_first_responder as extern "C" fn(&Object, Sel) -> BOOL,
        );
        declaration.add_method(
            sel!(acceptsFirstMouse:),
            accepts_first_mouse as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
        declaration.add_method(
            sel!(mouseDown:),
            mouse_down as extern "C" fn(&Object, Sel, id),
        );
        declaration.add_method(sel!(keyDown:), key_down as extern "C" fn(&Object, Sel, id));
        declaration.register()
    }

    fn frame(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    unsafe fn pump_main_run_loop() {
        // A bare NSRunLoop pump does not distribute WindowServer activation events.
        // Exercise NSApplication's real event loop, including key-window transitions.
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let until: id = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.01_f64];
        let event: id = msg_send![app, nextEventMatchingMask: u64::MAX untilDate: until inMode: NSDefaultRunLoopMode dequeue: YES];
        if event != nil {
            let _: () = msg_send![app, sendEvent: event];
        }
        let _: () = msg_send![app, updateWindows];
    }

    unsafe fn parent_window(app: id) -> Owned {
        let window = Owned(
            NSWindow::alloc(nil).initWithContentRect_styleMask_backing_defer_(
                frame(40.0, 40.0, 320.0, 180.0),
                NSWindowStyleMask::NSTitledWindowMask,
                NSBackingStoreBuffered,
                NO,
            ),
        );
        assert!(window.0 != nil, "native AppKit window allocation failed");
        let _: () = msg_send![window.0, setReleasedWhenClosed: NO];
        let _: () = msg_send![window.0, makeKeyAndOrderFront: nil];
        let _: () = msg_send![app, activateIgnoringOtherApps: YES];
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let key: BOOL = msg_send![window.0, isKeyWindow];
            if key != NO {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "AppKit fixture requires a real WindowServer/key window; refusing a false positive"
            );
            pump_main_run_loop();
        }
        window
    }

    unsafe fn add_above(parent: id, view: id) {
        let _: () = msg_send![parent, addSubview: view positioned: NSWindowOrderingMode::NSWindowAbove relativeTo: nil];
    }

    unsafe fn native_mouse_event(window: id, point: NSPoint) -> id {
        let number: isize = msg_send![window, windowNumber];
        msg_send![class!(NSEvent), mouseEventWithType: 1_u64 location: point modifierFlags: 0_u64 timestamp: 1.0_f64 windowNumber: number context: nil eventNumber: 1_isize clickCount: 1_isize pressure: 1.0_f32]
    }

    unsafe fn native_key_event(window: id) -> id {
        let number: isize = msg_send![window, windowNumber];
        let characters = Owned(NSString::alloc(nil).init_str("a"));
        msg_send![class!(NSEvent), keyEventWithType: 10_u64 location: NSPoint::new(80.0, 80.0) modifierFlags: 0_u64 timestamp: 2.0_f64 windowNumber: number context: nil characters: characters.0 charactersIgnoringModifiers: characters.0 isARepeat: NO keyCode: 0_u16]
    }

    unsafe fn record(condition: bool, failures: &mut Vec<&'static str>, reason: &'static str) {
        if !condition {
            eprintln!("FAIL: {reason}");
            failures.push(reason);
        }
    }

    pub unsafe fn run() {
        let main_thread: BOOL = msg_send![class!(NSThread), isMainThread];
        assert!(
            main_thread != NO,
            "AppKit regression must run on the actual process main thread"
        );
        let pool = NSAutoreleasePool::new(nil);
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let activated: BOOL = msg_send![app, setActivationPolicy: 0_isize];
        assert!(
            activated != NO,
            "AppKit fixture cannot become a foreground application"
        );
        let _: () = msg_send![app, finishLaunching];
        let parent = parent_window(app);
        let content: id = msg_send![parent.0, contentView];
        let allocated: id = msg_send![receiver_class(), alloc];
        let receiver = Owned(allocated.initWithFrame_(frame(0.0, 0.0, 320.0, 180.0)));
        content.addSubview_(receiver.0);
        let point = NSPoint::new(80.0, 80.0);

        // Negative control reproduces the exact old constructor and z-order.
        let legacy = Owned(NSView::alloc(nil).initWithFrame_(frame(40.0, 40.0, 160.0, 100.0)));
        legacy.0.setWantsLayer(YES);
        add_above(content, legacy.0);
        let hit: id = msg_send![content, hitTest: point];
        assert_eq!(
            hit, legacy.0,
            "negative control did not reproduce the legacy interception"
        );
        println!("PROOF legacy bare NSView intercepts the actual AppKit hitTest");
        let _: () = msg_send![legacy.0, removeFromSuperview];

        let mut failures = Vec::new();
        let render =
            Owned(crate::policy::new_render_view(frame(40.0, 40.0, 160.0, 100.0)).unwrap());
        render.0.setWantsLayer(YES);
        add_above(content, render.0);
        let hit: id = msg_send![content, hitTest: point];
        record(
            hit == receiver.0,
            &mut failures,
            "render-only surface intercepted the underlying native input view",
        );
        let direct_hit: id = msg_send![render.0, hitTest: point];
        record(
            direct_hit == nil,
            &mut failures,
            "render view itself must decline AppKit hitTest",
        );
        let accepts: BOOL = msg_send![render.0, acceptsFirstResponder];
        record(
            accepts == NO,
            &mut failures,
            "render view must not accept first responder",
        );
        let becomes: BOOL = msg_send![render.0, becomeFirstResponder];
        let key_view: BOOL = msg_send![render.0, canBecomeKeyView];
        record(
            becomes == NO && key_view == NO,
            &mut failures,
            "render view must refuse direct first-responder and key-view eligibility",
        );

        MOUSE_DOWN.store(0, Ordering::SeqCst);
        let event = native_mouse_event(parent.0, point);
        assert!(event != nil, "AppKit mouse event construction failed");
        let _: () = msg_send![parent.0, sendEvent: event];
        record(
            MOUSE_DOWN.load(Ordering::SeqCst) == 1,
            &mut failures,
            "actual NSWindow mouse dispatch did not reach the underlying view",
        );

        let accepted: BOOL = msg_send![parent.0, makeFirstResponder: receiver.0];
        assert!(
            accepted != NO,
            "fixture underlying view cannot become first responder"
        );
        let first: id = msg_send![parent.0, firstResponder];
        record(
            first == receiver.0,
            &mut failures,
            "native input view did not retain keyboard focus beneath the render surface",
        );
        KEY_DOWN.store(0, Ordering::SeqCst);
        let event = native_key_event(parent.0);
        assert!(event != nil, "AppKit key event construction failed");
        let _: () = msg_send![parent.0, sendEvent: event];
        record(
            KEY_DOWN.load(Ordering::SeqCst) == 1,
            &mut failures,
            "actual NSWindow key dispatch did not reach the underlying first responder",
        );

        render.0.setFrameOrigin(NSPoint::new(60.0, 50.0));
        render.0.setFrameSize(NSSize::new(120.0, 90.0));
        add_above(content, render.0);
        let hit: id = msg_send![content, hitTest: point];
        record(
            hit == receiver.0,
            &mut failures,
            "moving/reattaching the surface reintroduced input interception",
        );

        for child in [true, false] {
            let overlay =
                Owned(crate::policy::new_overlay_window(frame(40.0, 40.0, 160.0, 100.0)).unwrap());
            let _: () = msg_send![overlay.0, setReleasedWhenClosed: NO];
            if child {
                let _: () = msg_send![parent.0, addChildWindow: overlay.0 ordered: NSWindowOrderingMode::NSWindowAbove];
            }
            let _: () = msg_send![overlay.0, orderFront: nil];
            let ignores: BOOL = msg_send![overlay.0, ignoresMouseEvents];
            let key_allowed: BOOL = msg_send![overlay.0, canBecomeKeyWindow];
            let main_allowed: BOOL = msg_send![overlay.0, canBecomeMainWindow];
            record(
                ignores != NO,
                &mut failures,
                "native render overlay window accepts mouse events",
            );
            record(
                key_allowed == NO && main_allowed == NO,
                &mut failures,
                "native render overlay can become a key/main window",
            );
            let _: () = msg_send![overlay.0, makeKeyWindow];
            let _: () = msg_send![overlay.0, makeMainWindow];
            pump_main_run_loop();
            let actual_key: id = msg_send![app, keyWindow];
            let actual_first: id = msg_send![parent.0, firstResponder];
            record(
                actual_key == parent.0 && actual_first == receiver.0,
                &mut failures,
                "render overlay stole actual keyboard focus from the input window",
            );
            let _: () = msg_send![overlay.0, orderOut: nil];
            if child {
                let _: () = msg_send![parent.0, removeChildWindow: overlay.0];
            }
            let _: () = msg_send![overlay.0, close];
        }
        let _: () = msg_send![parent.0, orderOut: nil];
        let _: () = msg_send![parent.0, close];
        drop(render);
        drop(receiver);
        drop(legacy);
        drop(parent);
        let _: () = msg_send![pool, drain];
        assert!(
            failures.is_empty(),
            "native AppKit input routing regression failed: {failures:?}"
        );
        println!("PASS native AppKit hitTest, mouse/key delivery, move, responder refusal, and child/top overlay focus policy");
    }
}

#[cfg(target_os = "macos")]
fn main() {
    unsafe {
        proof::run();
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "This proof requires actual macOS AppKit; other platforms cannot produce a passing result."
    );
    std::process::exit(2);
}
