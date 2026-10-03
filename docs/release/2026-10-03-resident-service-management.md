# Windows resident service management — 2026-10-03

## Delivered

- Background autostart reads and updates the actual `MiniRemoteDesktop` Windows service configuration. Disabling autostart changes it to manual start and preserves the remaining service configuration. Changes are acknowledged only after readback succeeds.
- Graceful, after-sessions and force shutdown close admission to new sessions, acknowledge the request, and perform bounded cleanup. The management connection remains available during cleanup. Native UI shutdown and restart wait for the old process to exit and report failures.
- Closing the UI keeps the background service running. Settings exposes real autostart status and a separate exit-and-stop action.
- A dedicated local management pipe permits only health, status, autostart read/write and shutdown. It does not expose files, session creation or privileged execution through the interactive-user permission.
- The installer stops the old service before replacing binaries, preserves existing startup configuration, backs up the binaries and service configuration, and rolls back on upgrade failure. Install it together with `mrd-service-management.ps1`.
- Startup is reported only after both IPC endpoints bind successfully. Agent cleanup terminates surviving processes and supports cancellation during shutdown.

## Verification

| Check | Result |
| --- | --- |
| Service library, serial run | 638 passed, 5 explicitly ignored lab tests |
| Focused autostart / readiness / management / shutdown / web bridge / Windows service tests | 33 passed, 1 optional live test ignored in this run |
| Additional read-only check against the installed SCM service | Passed; actual startup mode Automatic |
| Windows IPC tests | 19 passed, including native interactive-user pipe access and first-instance collision rejection |
| Unix IPC tests in an isolated Tencent directory | 15 passed |
| Frontend | 729 passed across 57 files; type check and production build passed |
| Native UI service manager | 13 passed through the actual app Cargo test target |
| Installer helper | 5 passed; installer parse and dry-run passed |
| Production builds | Rdesk, mrd-service and mrd-session-agent completed |
| Independent final review | No remaining P1/P2 findings in this scope |

Two existing tests with short timing deadlines failed when the full service library ran concurrently with a release build. The focused check and the complete serial library run passed; no deadlines were widened.

## Windows update package

Package directory: `L:\project\mini-remote-desktop\target\resident-service-20261003`.

The package contains `Rdesk.exe`, `mrd-service.exe`, `mrd-session-agent.exe`, both installer scripts, and a SHA256 manifest. The native build includes the three preserved local edits in `main.rs`, `windows_pipe.rs` and `windows_sessions.rs`; those edits remain outside this feature commit.

Run from an **administrator PowerShell** in the package directory:

```powershell
.\install-mrd-service.ps1 -SourceDirectory $PWD.Path -Confirm:$false
```

Then run the packaged `Rdesk.exe`. Existing service startup configuration is preserved by the update. Service binaries are installed under `C:\Program Files\MiniRemoteDesktop`; machine state remains under the protected ProgramData directory.

At packaging time the local installed service was still the previous executable. Installation requires Windows administrator elevation; a completed build alone does not activate the update.

## Tencent Cloud

The backend and realtime units were independently checked: both active and enabled; both loopback health endpoints returned `status=ok`. The realtime health contract remained protocol 3, compatible with versions 2 and 3.

This release adds Windows resident management and shared IPC behavior. It does not require replacing or restarting the existing FastAPI and realtime executables. Unix IPC validation used an isolated directory under `/home/ubuntu/projects/.deployment/audits/resident-management-20261003`, separate from production sockets.

Direct-first public-network NAT traversal, TURN acceptance and account provisioning retain the limitations in the earlier [server connectivity deployment](2026-10-03-server-connectivity-deployment.md).
