# Existing Store Read-only Preflight Implementation Plan

> REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Verify the actual protected existing database with the candidate before any service stop, without migration, bootstrap, resealing, or production data changes.

**Architecture:** A public storage API opens SQLite with READ_ONLY and verifies the complete existing snapshot in one Deferred read transaction. The Windows service binary routes the exact sole argument --verify-existing-store before logging, SCM, or runtime initialization; it uses the installed-service ACL policy, protected product directory, platform secret protector, and fixed security-state-v2.sqlite3 filename. The successful JSON contains only format_version and identity_initialized.

**Tech Stack:** Rust, rusqlite, existing sealed storage verifier, Windows product directory ACL and DPAPI adapters.

## Task 1: Read-only storage verification

Files: crates/mrd-store-sqlite/src/lib.rs; crates/mrd-store-sqlite/tests/read_only_verification.rs.

1. Add real SQLite tests for existing v3 (initialized and uninitialized), genuine sealed v2 without migration, committed WAL visibility, missing/empty/future/tampered stores and wrong protector. Compare DB and WAL bytes around every probe.
2. Run the focused test via the task Cargo runner. Expect missing verify_existing_read_only API.
3. Add a nonsecret summary and READ_ONLY + Deferred verifier; validate only versions 2/3, reuse verify_store_snapshot_connection, commit the read transaction. Do not call configure/open/migrations/bootstrap/write_meta.
4. Run focused and complete mrd-store-sqlite tests.

## Task 2: Fixed Windows service entrypoint

Files: apps/mrd-service/src/main.rs.

1. Add argument routing tests that accept the exact sole flag, reject duplicate/extra/service arguments with it, and preserve ordinary service/console routing.
2. Observe the new route tests fail before implementation.
3. Route before initialize_logging. Build policy via installed_service(MRD_WINDOWS_SERVICE_SID); verify protected product directory; create platform protector; call fixed-file storage API; emit only the small JSON and exit.
4. Run binary argument tests and production service checks. Actual elevated candidate preflight against the live original path belongs to root after independent package review, outside this Rust implementation.

No commit/push or installation in this subtask; root integrates only after independent review.
