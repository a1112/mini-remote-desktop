# Legacy Temporary Machine Key Binding Implementation Plan

> For implementation: follow the existing systematic-debugging, test-driven-development, and verification-before-completion workflow.

**Goal:** Allow an already registered physical device to publish its temporary access password without user login, preserving its device code, account owner, machine key, credentials, and authorization version.

**Architecture:** Extend the device-authenticated temporary publication use case with a first, immutable machine key pin for legacy devices. Validate the contextual Ed25519 signature and authenticated device snapshot before adding a missing mapping in the same transaction as the publication. Use the existing principal key transaction lock before the physical device row lock, so physical and browser identities cannot race into sharing a key.

**Tech Stack:** FastAPI, SQLAlchemy, PostgreSQL, Ed25519, pytest.

## Observed failure

After service 54fc28be was installed, the actual API journal recorded successful temporary metadata GETs and rejected POSTs (401). A PostgreSQL read-only transaction for device 1501515774 confirmed physical=true, authentication active=true, account-bound=true, machine-key mapping=false, and temporary-access row=false. The backend currently rejects any publication lacking DeviceMachineIdentity; original tests insert that mapping directly before publishing and miss this compatibility case.

## Design decisions

The user has authorized anonymous remote control, temporary passwords, implementation, and deployment. This is a compatibility repair within that design. Keep the current signed publication protocol and resident service; do not issue a new device code or require account login.

An authenticated first publication provides both the existing physical device credential and proof of possession of the supplied private key. HTTP device credentials are already bearer credentials in the architecture; this addition must not bypass authentication or replace any pinned key. A new recovery challenge would require another client deployment without improving authentication of the existing device credential. Direct database repair would leave other legacy devices broken and is excluded.

First pinning is trust on first use authorized by the current physical bearer credential and possession of the new key. DeviceMachineIdentity also authorizes later key-based credential recovery; this is a durable authorization addition for legacy devices, not verification of a previously trusted key. Reject first pinning with an enabled=false document, which has no freshness field. Require the enabled publication expiry check before any mapping insert.

Check in order: key identifier/public key format and contextual signature; strict signed document; acquire existing principal key lock; reject browser-controller key reuse; lock the authenticated physical device row; revalidate every snapshot field; verify document device ID/auth version, scopes, and valid expiry; inspect mapping by device/key. If an exact mapping already exists, keep it. If the device has a different key, or the key belongs to another physical device, return the same generic rejection. Only a fully valid first publication may insert a missing mapping.

Insert and publication share one database transaction. Failures roll back both. Use the existing transaction-scoped cross-process key lock and physical-row serialization. Do not lock another device after locking this one; unique constraints remain the final defense and any conflict fails closed. Re-evaluate time after waits. Preserve all device fields, refresh tokens, ownership, tenant, authentication counters, and existing session authorization. Passwords and verifier-bearing documents remain out of logs and errors. Existing local-consent requirements remain in force.

## Task 1: Reproduce the compatibility failure

**Files:** apps/Rdesk-Server/tests/test_guest_browser_session_api.py, plus a focused compatibility test module if isolation improves readability.

Create a real API test for a legacy physical device without DeviceMachineIdentity. Build a valid signed publication using the existing test key and supply only the physical device header (no account token). Assert successful publication, exactly one mapping, and unchanged code/owner/tenant/auth version. Run this test before implementation and preserve its actual 401 failure. The helper must not silently preinsert a mapping.

## Task 2: Implement immutable first pinning

**Files:** apps/Rdesk-Server/app/services/temporary_access.py.

Reuse app.services.device_principal_keys principal_key_lock and key_is_browser_controller. Place the existing publication body under the shared principal lock before row locks, validate the complete proof/document/snapshot and expiry, then allow an absent physical mapping to be inserted. Reject existing device or key conflicts without overwriting any mapping. Maintain generic errors and one transaction, and retain exact retry/generation behavior. No schema migration or resident-service change is required.

## Task 3: Meaningful negative and concurrency coverage

Exercise bound and unbound physical devices, ordinary user/guest/browser credentials rejected, bad signature/expired or wrong-device document rejected with no mapping, existing different pin rejected, key already pinned to another device rejected, browser-controller key rejected, and exact retry idempotence. Verify failed publication does not commit a pin. Cover first-pin concurrency using the existing shared principal lock and row-lock conventions, plus PostgreSQL concurrency if the configured test environment supports it. Do not label SQLite coverage as PostgreSQL row-lock evidence.

Run the focused RED/GREEN cases, then the existing guest API, self-enrollment, device authorization/binding, browser-controller isolation, and temporary-authority tests appropriate to the change. Run full backend checks once if the configured environment permits. Preserve actual exit codes and review the final diff independently.

## Task 4: Deploy and observe through the formal API

Commit and push only the reviewed source, tests, and this plan. Prepare a new pinned backend source release; preserve the existing private environment, signing identities, database, and realtime binary/process. No SQL mutation or one-off rebootstrap is allowed. Build and verify before switching the API service; use the existing health-checked rollback pattern.

After deployment, let the installed resident service publish normally. Require actual POST200 and read-only TemporaryAccessStatus.ready=true, then inspect the native masked password card without writing its value into diagnostics. Verify mapping and device invariants using a read-only transaction. Only claim anonymous remote control acceptance after a real consented media and input session. Mac host unavailability remains an external acceptance dependency.
