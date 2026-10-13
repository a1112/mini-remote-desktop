# Agent Capture Failure Diagnostic Implementation Plan

**Goal:** Preserve the exact finite cause of an abnormal installed Agent capture-worker exit for the next actual browser connection.

**Architecture:** A private `capture_diagnostics` module publishes four bounded public fields to the existing Windows Application source, Event 3. Calls use the exact worker session and actual process ID; no new IPC or access authority is introduced.

**Tech Stack:** Rust, existing windows crate EventLog APIs, Cargo unit and authorization integration tests, reviewed Windows binary upgrade transaction.

## Task 1: Diagnostic record and stop semantics

Files: create `apps/mrd-session-agent/src/capture_diagnostics.rs`; wire it privately from `src/lib.rs`; add only `Win32_System_EventLog` to its Windows dependency features.

Write failing tests first for fifteen finite causes, exact four-field records, canonical UUID rejection, stop suppression, and sink failure leaving the caller's result unchanged. Run `cargo test --locked --offline -p mrd-session-agent --lib capture_diagnostic -- --nocapture`, retain actual assertion failures separately from compilation errors. Implement the smallest finite enum and bounded renderer/sink. Run the same tests to verify green.

## Task 2: Preserve original worker branches

Files: `apps/mrd-session-agent/src/capture.rs` and `src/runtime.rs`.

Replace only existing static reporting calls and add reporting at the two previously silent abnormal branches. Pass the existing worker session and stop flag. Retain each original return/break, worker queue, profile, payload cap, scaling, codec, and resource check. Keep normal stop silent. Add tests triggering the actual runtime pump's binding rejection, IPC conversion rejection, and outbound enqueue failure, then add finite reporting without changing the original errors. Do not guess session identity when desktop or event-context resolution fails before a trusted unit is available. Review every abnormal exit against the fifteen causes; explicitly record the native-panic and early-runtime coverage limits.

## Task 3: Verify and review

Run Agent library tests and `media_grants`, `input_grants`, `consent_routing` integration tests. Run formatting and `git diff --check`. Preserve raw command output, exit codes, source hashes, and RED/GREEN evidence. An independent agent reviews source and field privacy before root commits and builds the candidate.

## Task 4: Build and install the Agent candidate

Build exactly `cargo build --release --locked --offline -p mrd-session-agent --bin mrd-session-agent` from the clean reviewed commit. Stage and bind the PE/hash to that source, manifest, build log, and existing installed binaries. Derive an Agent-only protected upgrade transaction, retaining installed service c7b and UI 9297, identity, configuration, ACLs, and stopped-data commitment. Independently review the transaction before executing the already authorized update.

## Task 5: Reproduce and continue toward acceptance

Reconnect the UI through its normal lifecycle and repeat the 30 fps browser-to-local-device session through the existing product path. Record actual session UUID, original Agent PID, route proof, finite Event 2 and Event 3, and public health. Use the original worker cause to choose the next minimal regression/fix. Only after picture and keyboard/mouse work, measure 30/60/120 requested profiles with moving source and CPU/GPU observations.
