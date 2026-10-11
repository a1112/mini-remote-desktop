# Local Permission Status Implementation Plan

**Goal:** Display actual local screen and keyboard/mouse readiness immediately left of the title bar's connection-history button, as requested by the user.

**Architecture:** A native-only PermissionStatus component reads the existing local mrd-service capability snapshot. It never infers system permissions from session scopes or LAN advertisements. This feature does not change permissions, authentication, or attended access policy.

**Tech Stack:** React, TypeScript, existing Tauri IPC adapter, Vitest and Testing Library.

## Approved scope and design

The user explicitly requested a permission status display and clarified its location as “连接记录按钮左侧”. The compact control shows screen and keyboard/mouse states. Clicking it opens local system permission details and a read-only retry. Web clients hide it before any IPC call.

Map available/usable to 可用, permission_missing to 待授权, supported to 待检查, degraded to 受限, unsupported/unimplemented to 暂不支持, and unknown/invalid/missing/error to 未能确认. macOS uses capture.macos and control.keyboard_mouse. Other native platforms use their actual capture entry; static support cannot be shown as an actual successful permission probe. Display the snapshot time as a snapshot update, not an invented successful check timestamp.

## Task 1: Component and readiness mapping

Create apps/Rdesk/src/app/components/PermissionStatus.tsx and PermissionStatus.test.tsx. First verify meaningful failing tests for the native/web boundary, permission_missing vs static supported, failure clearing a prior usable state, retry, and late responses after unmount. Implement a guarded existing ipcCapabilitySnapshot call, bounded sequential refreshes to account for the service's cached response, focus refresh, and cleanup. Use existing theme and button/popover conventions; show no prompts or settings mutations.

Run node node_modules/vitest/vitest.mjs run src/app/components/PermissionStatus.test.tsx from apps/Rdesk. Do not install dependencies.

## Task 2: Placement

Modify apps/Rdesk/src/app/components/TitleBar.tsx at the connection-history action to render the new component immediately before that button. Verify the requested ordering and native/web behavior with an appropriate integration test. Keep small-window sizing and no-drag behavior intact.

## Task 3: Verification and delivery

Run affected component tests, related capability/adapter tests and node node_modules/typescript/bin/tsc --noEmit. Obtain an independent review. Build the production frontend after committing reviewed changes; preserve installed mrd-service binaries. Deploy the web timeout fix without exposing native-only permission controls. Build and switch native GUI candidates only with exact service hashes preserved, then visually verify location and real IPC states. Record any Mac system permission or session-display blocker rather than claiming remote media acceptance.
