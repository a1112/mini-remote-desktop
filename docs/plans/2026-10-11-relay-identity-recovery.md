# Relay Identity Recovery Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Recover the existing relay through formal APIs without changing its key or bypassing runtime validation, then continue dual-Windows WAN testing.

**Architecture:** A one-time Rust example runs as the relay service user to inspect and validate the original secure identity and atomically install a new certificate. A root-only Python operator script preserves backups and a recovery journal, uses protected local administrator credentials for formal enrollment/approval/pickup, and controls service isolation. Normal Agent owns subsequent secret rotation.

**Tech Stack:** Existing Rust mrd-relay-agent public interfaces, systemd credentials, Python standard library, existing relay API.

---

### Task 1: Certificate maintenance helper

Create `apps/mrd-relay-agent/examples/recover_identity.rs`. Use `SecureIdentityStore`, `HardenedAtomicFile`, `IdentityFsPort`, `load_or_create_identity`, `CertificateState`, and `SystemClock`. Refuse a missing original identity before any create-capable interface is called. Inspect returns original public CSR and non-secret recovery metadata to the root orchestrator. Install receives a certificate document from protected systemd credentials, preserves every original identity field except the active certificate, and validates a future renewal deadline. Check the original sealed hash and backup before the original atomic write; report only static errors.

Write tests first for real CA/key validation, key/epoch/sequence/pending state preservation, missing original/backup, stale hash, rejected certificates and atomic failure. Run the new tests red before implementation, then `cargo test --locked -p mrd-relay-agent --example recover_identity`. Run existing identity/secure-store tests and compile on Linux, which owns production persistence.

### Task 2: Root orchestration

Create `deploy/turn/linux/recover-expired-identity.py` and `deploy/turn/linux/test_recover_expired_identity.py`. First test protected no-follow root-owned file reads, configuration preservation, redacted API errors and resumable stage journal. Implement the minimum root orchestrator: verify unchanged expired node/configuration, isolate the failed Agent, preserve raw identity/runtime backups, capture service-user inspect output, use local admin API for enrollment token and approval, use trusted HTTPS for enrollment and receipt pickup, then install through the helper with original hash. Preserve journal after every durable action; do not reset database state or auto-start after failed validation.

Run `python -m unittest discover -s deploy/turn/linux -p test_recover_expired_identity.py`. Review combined implementation for preservation and secret redaction before production use.

### Task 3: Real recovery and WAN verification

Commit reviewed sources and transfer through Git into a dedicated server checkout. Build the maintenance example using the existing Linux cargo cache. Run the operator with the original node/configuration and protected existing credentials. Verify new certificate, preserved identity/runtime, rotation version 2 completion, valid lease and healthy admission from real service/database/audit state. Retain root-private recovery artifacts.

Continue the existing MS-LGKRSZGOVODD chat for target readiness, then create actual guest browser sessions and request real screen/input consent. Record actual ICE selection, 30 seconds of remote video with decoded/presented growth, authorized input ACK and cleanup. Management-overlay connectivity and configured policy labels do not count as project route evidence.

### Task 4: Correct the rotation attempt deadline exposed by real recovery

The approved recovery keeps the normal Agent responsible for the existing rotation. Production validation found that all four worker call sites wrap `advance_secret_rotation` with the 2-second local metrics deadline. A real fenced TURN probe takes longer; applying a secret can also involve a process restart and multiple authenticated requests. Cancellation leaves the durable Applied phase intact but prevents its normal proof and commit.

Use a dedicated bounded 60-second rotation attempt deadline at both production worker and once-worker call sites. Keep local metrics sampling at 2 seconds. Preserve every phase transition, safe credential window, generation/version check, certificate and request signature validation, actual allocation requirement, live probe, proof MAC, commit/status recovery and cancellation behavior. This changes the waiting budget for the existing transaction, without creating another rotation owner or bypass path.

First demonstrate a failing worker regression with a real asynchronous probe delay exceeding 2 seconds. Verify that the corrected worker completes the same signed commit and durable finalization. Also verify cancellation at the dedicated deadline preserves the resumable phase. Review the four call sites and run the relay runtime/identity/protocol tests on Windows and Linux before a backed-up deployment of the normal Agent.

The graceful-drain/metrics dependency loop and NetBird's insertion into foreign INPUT chains are separate findings. Their permanent fixes are outside this narrow change. This recovery uses independently reviewed, bounded maintenance through existing broker operations, preserves original secrets and journals, and reconciles only the exact extra rule inside the project-owned table. It never disables NetBird ACLs, edits identity/runtime envelopes, resets database fields or treats missing metric samples as trusted zero.

### Verified execution checkpoint (2026-10-11)

The original node `relay-tencent-gz-1`, key and identity epoch 102 were preserved during formal re-enrollment. The root-only operator and certificate helper passed their Linux checks; the original identity/runtime and audit history remain backed up privately. HTTPS recovery uses bounded curl with the original protected CA, without disabling certificate checks.

Commit `3fe75d5beaf9bc66b86c69e661984169909295fd` corrected the four transaction deadlines. Seven worker regressions failed with the original 2-second boundary and passed with 60 seconds. Windows passed 65 library and 95 runtime integration tests. Linux passed 59 library and 95 runtime integration tests; its normal release Agent built from the same clean commit. The Linux executable SHA256 is `218f689bb7a5a88ae5999eb31a8fe45f53860842e88dac014ee850cce61fdede`.

A separately reviewed root rollout backed up the original executable and current sealed state, atomically installed the verified executable, and started only the normal Agent. TURN remained at generation 11 with its existing invocation. At 19:18:19 +08:00 the backend recorded the original v2 rotation commit and its proof; active, applied and desired versions became 2 and pending rotation cleared. At 19:18:28 three healthy heartbeats restored available admission and a future lease. No database promotion or synthetic allocation proof was used.

At 19:23:43 the normal certificate renewal also completed: its audit count increased from 101 to 102, the original pending renewal promoted its next key and epoch 103, and the certificate lifetime extended to about one hour. Admission remained available after three healthy epoch-103 heartbeats, with secret version 2 and a future lease. Preserving the original key/epoch applies to the one-time recovery install; the subsequent normal renewal performs its existing key and epoch transition.

These results establish relay recovery. Actual dual-Windows ICE selection, sustained remote video, human-approved input and session cleanup remain pending. MS-LGKRSZGOVODD has a running registered/authenticated service and interactive UI; its temporary-access status query returns EOF, so the formal UI status still needs human verification. NetBird remains a management path and is not project WAN proof. Permanent graceful-drain/metrics and foreign nftables chain coexistence corrections remain separate work.
