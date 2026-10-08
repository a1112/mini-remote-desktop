# macOS controlled host

The macOS resident runs in the logged-in user's session, using the existing
Keychain-backed state, private Unix IPC, signed LAN authorization and native
ScreenCaptureKit/VideoToolbox pipeline. Incoming attended sessions require the
Rdesk window to stay running for local approval.

This update adds CoreGraphics keyboard and mouse injection with live
Accessibility/post-event permission checks. It maps protocol virtual keys to
native ANSI positions and supports pointer movement, dragging, five buttons,
click counts and pixel scrolling. Unsupported system/media keys and Unicode
text are rejected. Ending a session releases its held input and resets idle
cursor, click and Caps Lock caches without clearing another session's holds.

Remote video coordinates are mapped to the selected display/window's global
logical bounds, including Retina scaling and negative display origins. Native
queries run outside the async executor; expired events, changed sources and
invalid geometry are rejected. The default source and actual video dimensions
are resolved before the signed profile is issued, then retained through commit.
See Apple's [display bounds](https://developer.apple.com/documentation/coregraphics/cgdisplaybounds(_:))
and [window bounds](https://developer.apple.com/documentation/coregraphics/kcgwindowbounds?language=objc)
contracts.

Capability snapshots distinguish missing Screen Recording/Accessibility
permissions from an unsupported platform and refresh permission state on each
read. Service health describes process health; it alone does not prove that
capture, input, LAN discovery and the consent UI are ready.

## Run and verify

```sh
cd apps/Rdesk
pnpm install --frozen-lockfile
pnpm tauri:dev
```

From the repository root:

```sh
python3 scripts/macos/check-controlled-host.py
python3 scripts/macos/check-controlled-host.py --require-cloud
```

The read-only checker returns success only when service health, LAN discovery,
the consent UI and both native permissions are available. The cloud option also
requires an enrolled device and authenticated signaling. It does not issue
credentials or change macOS permissions. When required, grant Screen Recording
and Accessibility to the running Rdesk Service in macOS Privacy & Security.

After an ad-hoc signed service update, macOS may require authorization to read
the existing Keychain item. Choose **启动后台服务** in Rdesk or run the exact
service executable in user-start mode, then confirm the native prompt:

```sh
target/debug/MrdService.app/Contents/MacOS/mrd-service --authorize-keychain-and-run
```

The same service process prompts once and retains its authorized protector while
running. A single-read native approval therefore suffices for that process;
verification in a process that exits cannot authorize a later background read.
The separate `--authorize-keychain` command is an access check that exits;
persistent native permission is needed for a subsequent noninteractive process.
Ordinary background startup still disables Keychain interaction.

The registration modal and device service use the shared Tauri 2 hardware
adapter. They no longer call the obsolete `window.__TAURI__.invoke` API.

## Cloud connection

The configured API is `https://175.178.16.90/rdesk/api/v1` and signaling endpoint
is `wss://175.178.16.90/rdesk-realtime/ws`. The service automatically proves its
protected machine key using the server's signed challenge protocol and obtains
a device code and renewable credentials. Rdesk requires no user-entered
enrollment credential. Protected device credentials remain in the resident.
The server's SSH management key is separate from the machine signing key.

New device codes are nine decimal characters. The server derives a first
candidate from the keyed hardware-identity digest modulo `10^9`, pads leading
zeroes, and enforces database uniqueness. Collisions use bounded random
candidates in the same nine-digit space. Existing assignments are returned
from their persisted binding on retry, refresh or signed key recovery.
Expired device credentials recover through a signed request constrained to the
saved `expected_device_id`; missing mappings and mismatched codes cannot turn
recovery into a new allocation, and revoked devices remain denied.

macOS uses its platform UUID. New Windows/Linux installations prefer firmware
UUID or a valid board serial, with installation identity/protected key fallback.
A saved hardware-bound identity must still match the current motherboard before
its code is reused. Existing OS-based registrations preserve their assigned
identity. Hardware probing on Windows and Linux still needs native acceptance.

On 2026-10-08, SSH authentication and both public HTTPS health checks succeeded;
the API and realtime systemd services were active. The cloud's nine-digit
allocation update passed live signed registration: a fresh code had nine digits,
a fresh challenge with the same key/serial returned that code, and a different
key claiming the same serial was rejected with HTTP 409. The synthetic unbound
validation device was removed, retaining the audit record. Deployment backup:
`/home/ubuntu/projects/.deployment/updates/rdesk-nine-digit-20261008-174475cc/backup`.
The recovery guard backup is
`/home/ubuntu/projects/.deployment/updates/rdesk-recovery-guard-20261008/backup`.
Live signed recovery retained the original nine-digit code, while wrong expected
codes and recovery without a machine mapping both returned HTTP 409.

The local Mac still requires native Keychain authorization after the development
signature update. Native desktop input
delivery and a real Mac-target remote session still require acceptance with a
controller; deterministic tests do not certify those interactions.
