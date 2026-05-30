# Input Capture and Injection Design

Date: 2026-05-30

## Goal

Implement keyboard and mouse capture, transport, and injection for remote desktop sessions, using RustDesk as a reference for the shape of the feature while keeping this repository's thin UI shell plus service-owned session architecture.

## Current State

The shared control protocol already defines keyboard and mouse events:

- `MouseMove`, `MouseButton`, `MouseWheel`
- `Key`
- `ChannelClass::Realtime` for move and wheel events
- `ChannelClass::Reliable` for keys and mouse buttons

The active product does not yet wire those events end to end. The service advertises `control.keyboard_mouse` as unimplemented, the remote display UI does not capture pointer or keyboard input, and the native render surface has no input forwarding path.

## Reference Points From RustDesk

RustDesk separates client-side input capture from controlled-side injection. The controlled side owns platform injection, tracks modifier and lock-key state, releases pressed input on focus/session transitions, and uses platform-specific paths when normal injection is insufficient. On Linux Wayland, it uses a uinput-style virtual device path. On Windows, normal desktop injection uses the platform input API through an abstraction.

The important behavior to reuse here is not RustDesk's exact code, but the architecture:

- capture and injection are separate responsibilities;
- controlled-side injection is service-owned;
- keys and button transitions are reliable;
- pointer motion can use a lower-latency lossy lane;
- pressed state must be cleaned up when focus or sessions end.

## Chosen Approach

Use a Windows-first full chain.

This matches the current performance-critical stack in this repository: DXGI capture, D3D11 render, NVENC, and NVDEC are already Windows-centered. The code will still expose platform-neutral types and traits, but only Windows will advertise working injection until other platform adapters exist.

Alternatives considered:

1. Protocol and UI skeleton only. This would be fast, but it would not satisfy real keyboard/mouse injection.
2. Cross-platform implementation from day one. This would require Windows SendInput, macOS CGEvent, X11 XTest, and Linux Wayland uinput/portal handling in one pass, which is too broad for this branch.

## Components

### `crates/mrd-input`

Add a new Rust crate for platform-neutral input control.

Responsibilities:

- define service-facing input event and button/key types;
- define an `InputInjector` trait;
- provide a recording injector for unit tests;
- provide a Windows `SendInput` injector behind `cfg(windows)`;
- provide explicit unsupported adapters for non-Windows platforms.

The crate must track pressed keys and buttons so callers can release all active inputs during blur, window close, permission revoke, or session stop.

### `mrd-service`

The service remains the owner of input injection.

Responsibilities:

- probe whether platform injection is available;
- advertise `control.keyboard_mouse` as available only when the injector works;
- accept control input requests through IPC and, later, transport receivers;
- maintain `ControlChannelSnapshot` counters for sent, received, injected, failed, stale, and dropped events;
- release active input on session stop and error paths.

### `common-control-proto`

Keep the existing protocol shape. Add tests or helpers only where required to make lane selection, validation, and frame conversion explicit.

Expected lane policy:

- realtime: mouse move and wheel;
- reliable ordered: key down/up and mouse button down/up.

### UI capture in `apps/Rdesk`

The remote display page captures input only when the render area is focused and the session allows control.

Responsibilities:

- make the render area focusable;
- capture pointer move, pointer button, wheel, key down, and key up;
- map render-area coordinates to remote image coordinates;
- release active keys/buttons on blur and unmount;
- dispatch typed input events through the Tauri adapter.

The UI must not advertise control as active unless service capabilities allow it.

### Native render surface

The Windows native child HWND used by D3D11 rendering can receive mouse and keyboard events before React sees them. The Windows surface WndProc must forward input to the same service-owned control path. This keeps native rendering from disabling remote control.

The first implementation should cover Windows messages for:

- `WM_MOUSEMOVE`
- button down/up messages
- `WM_MOUSEWHEEL`
- `WM_KEYDOWN`, `WM_SYSKEYDOWN`
- `WM_KEYUP`, `WM_SYSKEYUP`
- focus loss cleanup

## Data Flow

Controller side:

1. Remote display UI or native render surface captures input.
2. Capture code normalizes input into shared control event fields.
3. The Tauri adapter sends the event to the local service.
4. The service selects realtime or reliable lane using `ControlEvent::channel_class()`.
5. Transport sends the frame to the controlled peer.

Controlled side:

1. The service receives a control frame.
2. The frame is decoded and sequence-checked per lane.
3. The service validates permissions and session state.
4. The platform injector applies the event.
5. Counters and last error are updated.

## Safety and Policy

Input control is disabled unless all of these are true:

- the session role allows controlling the peer;
- the peer capability advertises keyboard/mouse control;
- the session is connected and not view-only;
- the local service has an available injector on the controlled side.

Pressed keys and buttons are released on:

- render-area blur;
- native surface focus loss;
- session stop;
- capability revoke;
- injector error that leaves state uncertain.

## Testing Strategy

Use TDD for implementation.

Initial required tests:

- `common-control-proto`: lane selection remains reliable for keys/buttons and realtime for move/wheel.
- `mrd-input`: recording injector records events and releases active input; unsupported adapter reports unavailable; Windows mapper converts events to injection commands.
- `mrd-service`: capability status follows injector availability; input request updates counters and returns errors on unavailable injection.
- `apps/Rdesk`: remote display maps coordinates and dispatches pointer, wheel, and keyboard events only when focused/control-enabled.

Verification after implementation:

- `cargo fmt --check`
- `cargo test -p common-control-proto`
- `cargo test -p mrd-input`
- `cargo test -p mrd-ipc`
- `cargo test -p mrd-service`
- `pnpm --dir apps/Rdesk test -- <input-related tests>`
- `pnpm --dir apps/Rdesk type-check`

## Non-Goals For This Branch

- Full macOS and Linux injection.
- Wayland uinput service implementation.
- Public internet relay protocol changes beyond typed control frames required by active transports.
- Gesture-specific mobile input.
