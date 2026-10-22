#![cfg(target_os = "macos")]

use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use mrd_input::{macos::MacosInputInjector, InputButton, InputEvent, InputInjector, InputKey};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Observe only MRD-tagged events and turn them into null events before they can
/// type/click into the user's applications. Requires the real Accessibility grant.
#[test]
#[ignore = "requires an unlocked macOS desktop and Accessibility permission"]
fn native_keyboard_pointer_drag_and_wheels_reach_the_macos_event_stream() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let observed = received.clone();
    let tap = CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![
            CGEventType::MouseMoved,
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGEventType::LeftMouseDragged,
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::ScrollWheel,
        ],
        move |_, kind, event| {
            if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == 0x4D524449 {
                observed.lock().unwrap().push((
                    kind as u32,
                    event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
                    event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1),
                    event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2),
                ));
                return CallbackResult::Drop;
            }
            CallbackResult::Keep
        },
    )
    .expect("create isolated MRD input event observer");
    let source = tap.mach_port().create_runloop_source(0).unwrap();
    let run_loop = CFRunLoop::get_current();
    unsafe {
        run_loop.add_source(&source, kCFRunLoopDefaultMode);
    }
    tap.enable();
    let worker = std::thread::spawn(|| {
        let mut injector = MacosInputInjector::new();
        for event in [
            InputEvent::MouseMove { x: 120, y: 140 },
            InputEvent::MouseButton {
                button: InputButton::Left,
                pressed: true,
            },
            InputEvent::MouseMove { x: 130, y: 150 },
            InputEvent::MouseButton {
                button: InputButton::Left,
                pressed: false,
            },
            InputEvent::Key {
                key: InputKey::VirtualKey(0x41),
                pressed: true,
            },
            InputEvent::Key {
                key: InputKey::VirtualKey(0x41),
                pressed: false,
            },
            InputEvent::MouseWheel { delta: 120 },
            InputEvent::MouseHorizontalWheel { delta: -120 },
        ] {
            injector.inject(&event).unwrap();
            std::thread::sleep(Duration::from_millis(30));
        }
    });
    let deadline = Instant::now() + Duration::from_secs(4);
    while received.lock().unwrap().len() < 8 && Instant::now() < deadline {
        unsafe {
            CFRunLoop::run_in_mode(kCFRunLoopDefaultMode, Duration::from_millis(50), true);
        }
    }
    worker.join().unwrap();
    unsafe {
        run_loop.remove_source(&source, kCFRunLoopDefaultMode);
    }
    let events = received.lock().unwrap();
    assert_eq!(events.len(), 8, "observed native events: {events:?}");
    assert_eq!(events[2].0, CGEventType::LeftMouseDragged as u32);
    assert_eq!(events[4].1, 0, "wire A maps to macOS keycode 0");
    assert_eq!(events[6].2, 1);
    assert_eq!(events[7].3, -1);
    println!(
        "native input: move, click/release, drag, key down/up, vertical/horizontal wheel: 8/8"
    );
}
