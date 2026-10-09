# Database V3 Compatibility Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Open the existing sealed v3 store in the current anonymous remote-control client, preserving keys, permission ceilings, audit history and monotonic counters.

**Architecture:** Integrate established d07f9ccfc6067eb920385ec4fb4143016b7f8f1a v3 and LAN-pairing hunks into b32e987917c6d980cbe8b468c4dff2822453fcb1. Retain b32 temporary-password/WAN functionality and stricter macOS IPC caller checks. Never downgrade, delete, rebootstrap or restore the production database.

**Tech Stack:** Rust 1.94.1, SQLite, Windows DPAPI, TypeScript/React, Vitest; thin UI shell and local service.

## Evidence

The old executable ran against this database. The candidate failed with actual Windows event `unsupported database schema version 3`. Old executable SQL contains trust_permissions/schema3. Historical source provenance did not establish actual database compatibility. The failed installer remains immutable.

## Task 1: Store regression and compatibility

Files: crates/mrd-store-sqlite/src/{lib,integrity,migrations,trust_store}.rs; tests/policy_persistence.rs, tests/trust_policy.rs, tests/fixtures/sealed-v2.sql.

1. Import the genuine historical LF fixture and policy tests first. Run `cargo test -p mrd-store-sqlite --test policy_persistence -j 1`; preserve the expected missing-policy/compatibility failure.
2. Integrate the four historical files, retaining unrelated later fixes. Use `git diff d07f9ccf^ d07f9ccf -- crates/mrd-store-sqlite` as the exact implementation reference. Existing v3 validates without DDL/resealing. V2 authenticates the complete old snapshot before one immediate migration. Legacy trust gets no scopes.
3. Run `cargo test -p mrd-store-sqlite -j 1`. Validate reopened v3 and permission tampering, genuine v2 identity/protected-key/audit preservation, invalid/future versions without mutation, rollback, late-lock checks and concurrent exactly-once migration.

## Task 2: Preserve runtime permission enforcement and LAN pairing

Files: apps/mrd-service/src/app_state/{device_identity_registry,audit_log_registry}.rs; handlers/identity.rs; lan_discovery.rs; lan_discovery/{first_pairing,pairing_approval,peer_registry}.rs; session_authorization.rs; ipc_server.rs; ipc_server/{dispatch,product}.rs; crates/mrd-ipc/src/lib.rs. Tests: first_pairing_integration_tests.rs, policy/grant tests in those modules, crates/mrd-ipc/tests/lan_pairing_contract.rs.

1. Import signed-pairing and permission tests first; record meaningful red results.
2. Integrate d07 parent-to-commit hunks. Unknown/inactive/legacy empty ceilings deny. Incoming/outgoing LAN and controller grants intersect requested, persisted peer, machine and runtime ceilings.
3. Preserve b32 install_verified_wan_target_grant, temporary-access IPC arms, macOS peer identity and sensitive-command restrictions. Pair approval requires exact installed UI/active logon caller, candidate/nonce binding, two deadlines and late actor checks within the SQLite write transaction.
4. Preserve product audit ownership, outgoing birth receipt and last_sequence boundaries with pairing projection. Do not widen management IPC.
5. Run focused service LAN/session-authorization/product/caller tests, `cargo test -p mrd-ipc --test lan_pairing_contract -j 1` and production service checks.

## Task 3: Native pairing UI

Files: apps/Rdesk/src/app/adapters/tauri/{types,commands}.ts; components/{DevicesPage,LanPairingPanel}.tsx. Tests: commands.lanPairing.test.ts, DevicesPage.lanPairing.test.tsx, LanPairingPanel.test.tsx.

1. Add historical tests and record failures on the b32 baseline.
2. Integrate these four native UI changes while retaining the guest browser, temporary-password card and b32 automatic registration. Existing export-star adapter barrels need no extra file.
3. Run focused Vitest tests, TypeScript checking and native frontend build; run the full frontend suite once after stabilization.

## Task 4: Review and delivery

1. Independently review v3/permission enforcement and retained WAN/Mac boundaries.
2. Commit/push authorized changes. Build complete GUI/service/agent from exact reviewed source. Preserve immutable b32 candidates and failure evidence.
3. Before replacement prove actual persisted compatibility through a narrow protected diagnostic, including active store version, schema/manifest validation and actual DPAPI path; never infer it from an old Git label or print secrets.
4. Install after actual idle/handle/SCM/ACL proofs. Verify exact new hashes/full source stamp, service health, unchanged device code and masked temporary-password UI.
5. Public guest web entry is deployed. Mac video/input acceptance requires actual host connection and legitimate local approval; fabricate neither session nor consent.

## Parallel operational recovery

A separately sealed failed-generation recovery may restore exact old binaries only after finite SCM failure actions finish, exclusive image handles prevent new launch and a full quiescent census passes. It does not label census as the original-held exit proof. Preserve data/configuration/ACL. Root dispatches once after review and verifies actual health.
