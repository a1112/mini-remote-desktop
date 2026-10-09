# Independent Browser Remote Control Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** A browser with no local Rdesk/service can request attended remote access, receive real remote video, and send granted keyboard/pointer input.

**Architecture:** A bounded browser Controller principal joins the existing signed v3 authorization and WebRTC path. Python owns account access and ephemeral credentials, Rust signaling enforces the bound Controller identity, the remote resident remains responsible for consent/grants/media/input, and TypeScript owns browser peer lifecycle and rendering.

**Tech Stack:** FastAPI/SQLAlchemy, Rust mrd-signal-proto/realtime-server/mrd-service, React/TypeScript, WebCrypto Ed25519, WebRTC H.264, pytest/Vitest/Cargo.

User explicitly approved the independent browser design on 2026-10-09. Execute in this session using the attached settings-console-fix managed worktree, preserving the uncommitted settings work. No automatic commit, merge or production deployment; synchronize only guarded, reviewed changes. Root coordinates shared contracts before independently owned implementation tasks.

## Task 1: Browser identity and backend session APIs

Files: new browser schemas/services/routes and model under `apps/Rdesk-Server/app/`; focused tests under `apps/Rdesk-Server/tests/`; minimal required existing device-session/security/realtime/relay/listing integration.

1. Fix bootstrap/inspect/credential/relay/close response contracts with root and signaling owner before writing production code.
2. Write failing API/service tests for account/tenant-target authorization, public-key binding, bounded scopes/lifetime, Controller-only credentials, physical Device Bearer rejection, expired/revoked identity and original device compatibility.
3. Run focused pytest and observe the expected missing behavior.
4. Implement ephemeral BrowserController persistence/lifecycle and the browser-only API; never return physical device credentials. Retain target inspect/approve compatibility and exact v3 request commitment/grant semantics.
5. Rerun tests; check stale state, logout/close cleanup, device-list exclusion and credential redaction. Report commands, actual exit codes and contract to root.

## Task 2: Rust signaling and signed protocol compatibility

Files: `apps/realtime-server/src/{backend_token,auth,ws}.rs`, `crates/mrd-signal-server/src/lib.rs` or narrow supporting files as actually required; `crates/mrd-signal-proto` tests/fixtures only unless contract evolution proves necessary.

1. Agree browser credential claims/validation with Task 1. Keep ordinary signaling credentials backward compatible.
2. Write failing tests for browser controller registration proof, exact target/session forwarding, expiry/revocation, prohibited Agent/presence/other-session activity and invalid signatures.
3. Run focused Cargo tests to prove the missing behavior, then implement minimal bounded principal enforcement.
4. Provide cross-language deterministic v3 signing/commitment/control fixtures and exact field/context/framing definitions to root's TS protocol implementation.
5. Run protocol and sidecar tests/checks and report actual completed results.

## Task 3: Browser protocol and WebRTC engine (root)

Files: new `apps/Rdesk/src/app/services/browserRemoteProtocol.ts`, `browserRemoteSessionService.ts`, `browserRemotePeer.ts`, focused tests; minimal existing adapter utilities only if required.

1. Write failing interoperability tests from Rust-generated golden bytes for registration, signed request/offer/candidates/grant/control envelope and MRMX framing.
2. Implement bounded typed serialization, Ed25519 key lifetime/signing/verification, robust signaling queue and v3 state handling. Reject unknown/out-of-order/expired/wrong-peer messages.
3. Write failing peer lifecycle tests for consent/route/grant gating, ICE candidate queue, H.264 ontrack, first-frame readiness, scope checks, reliable input order, release and late-callback cancellation.
4. Implement real RTCPeerConnection and existing ctrl channel wire support; use the approved session's signed relay configuration and actual connection evidence. No same-page loopback fallback.
5. Run focused Vitest and TypeScript checking; report explicit unsupported browser capabilities instead of simulating success.

## Task 4: Browser remote page and connection integration

Files: new `apps/Rdesk/src/app/components/BrowserRemoteSessionPage.tsx`, focused page tests; `routes.ts`, `services/remoteDisplayLauncher.ts`/tests and only required auth logout integration/shared pure input tools.

1. Use root's service interface; write failing component tests for awaiting consent, real first-frame gate, scoped controls, visible API/signaling/media failures and cleanup.
2. Implement responsive video stage, connection/status/cancel, focusable keyboard surface, pointer mapping and fullscreen. Do not add unsupported toolbar actions.
3. Update browser launcher to request real target access and route the returned binding. Keep native launcher and explicit local-preview behavior intact.
4. Ensure logout closes only owned browser sessions and releases input; retain earlier theme/close/settings behavior.
5. Run focused page/launcher/auth tests, TypeScript checking and self-review. Root independently reviews specification then quality.

## Task 5: Target compatibility and actual integration

Files: `apps/mrd-service/src/wan_session/{executor,webrtc,media_runtime}.rs`, `transports/webrtc.rs` and targeted tests only where browser compatibility requires changes; integration evidence/tools under ignored `target/browser-remote-debug`.

1. Trace the exact existing controller workflow and handoffs to root's browser engine. Verify browser principal is bound by approved backend grant and signed key, not by caller-supplied labels.
2. Add failing compatibility tests before any required behavior changes. Preserve native device sessions and installed UI IPC boundaries.
3. Ensure H.264 RTP, consent/route activation, input envelope and lease cleanup work with the browser; never enable independent local capture preview for a remote target.
4. Run focused service checks/tests. Independently review integrated functionality and permissions.
5. Verify a real browser↔independent target flow, selected path and frame/input evidence where current device/permission state permits. Record concrete limitations instead of broad completion claims.

## Task 6: Final review, verification and source sync

1. Review spec coverage first, then code quality; fix findings with regression tests and re-review.
2. Run meaningful combined Python/Rust/frontend checks, typecheck and production build, capturing completed exit codes and counts.
3. Compare original project against a recorded pre-task byte manifest; copy only owned new/changed files with per-file conflict guards and backups. Keep unrelated files and earlier settings modifications.
4. Confirm exact source bytes, runtime artifact identity and startup; preserve existing remote sessions/services. No global process kills or identity/authentication bypass.
5. Deliver actual implemented behavior, local file links, measured evidence and any unverified real-device/network conditions.
