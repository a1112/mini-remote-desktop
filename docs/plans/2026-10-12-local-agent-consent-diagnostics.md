# Local Agent Consent Diagnostics Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Diagnose the installed Windows capture Agent readiness and attended-consent failure without changing authorization conditions.

**Architecture:** Extend the existing read-only ServiceHealth response with an optional bounded Agent readiness projection. Preserve the installed-product caller checks and consent-denial code; use only fixed, typed diagnostic messages at consent boundaries. No identity, credential, request payload, or raw error is exposed.

**Tech Stack:** Rust, serde, Windows active-console observation, existing AgentRegistry and authenticated product IPC.

## Evidence and choice

The c988 service and Agent are running in sessions 0 and 1, but process liveness does not prove registered capability readiness. Actual confirmation returned E_PRODUCT_DESKTOP_CONSENT_DENIED; product.rs currently collapses every prepare_consent error to one generic message. The actual production desktop-watcher probe in the current unlocked interactive desktop returned Default, epoch 2. This is a separate process probe and does not establish the installed Agent's state.

Bounded health projection and typed product error messages are preferred over persistent raw logging (unnecessary sensitive-data retention) or repeated connection attempts without readiness evidence. Prior user instructions authorize completing automatic local testing and fixing blockers without further permission requests.

## Task 1: Read-only readiness

Files: crates/mrd-ipc/src/lib.rs; apps/mrd-service/src/handlers/telemetry.rs; affected mrd-ipc ServiceStatus literal tests.

1. Add a failing serde round-trip test for readiness data; verify it fails because unknown readiness data is currently dropped.
2. Add optional AgentReadinessStatus to ServiceStatus with default and omit-None compatibility. Fields: console_active, registered, heartbeat_fresh, capture_advertised, consent_advertised, input_advertised, optional negotiated protocol_minor.
3. On Windows, derive the projection from WTSGetActiveConsoleSessionId and the registry's immediately observed active registration, health and advertised capabilities. Other platforms omit it. Do not infer desktop-unlocked from PID, and do not change healthy semantics or request allowlists.
4. Test missing registration, stale heartbeats, empty capabilities, supported capabilities, and wire backward compatibility.

## Task 2: Typed consent diagnostics

Files: apps/mrd-service/src/console_capture.rs; apps/mrd-service/src/ipc_server/product.rs.

1. Add failing tests asserting fixed messages for expired or missing consent, wrong desktop/logon, registration/capability readiness, native request timeout/revocation/unavailability, and an opaque fallback for arbitrary errors.
2. Attach typed static stage context to prepare_consent boundaries; project messages from these types and AgentRouteError/AgentRequestError enums. Preserve denial code, decision, deadlines, grant verification, all permission checks and exact-desktop binding.
3. Never surface arbitrary anyhow chains, paths, peer identities, native status strings, or cryptographic material.
4. Run relevant unit and installed-product boundary tests and obtain independent review.

## Task 3: Actual validation

1. Commit the reviewed source, build clean locked release products, and stage hash-pinned artifacts with the unchanged native UI provenance explicit.
2. Reuse the reviewed protected upgrade transaction, refresh pins and actual backups, validate, install and verify hashes, identity and configuration preservation.
3. Read actual ServiceHealth readiness before another short-lived browser request. Use the existing product UI and attended-consent flow. Once accepted, measure video playback counter growth and real Notepad pointer/keyboard effects, then 60/120 fps with measured constraints.
4. Do not treat unit tests, synthetic encoder throughput or process liveness as browser video/input acceptance.
