# Local browser performance implementation plan

**Goal:** Keep local browser video and input working through high capture rates, then reduce measurable processing cost.

**Architecture:** Retain the UI/service/Agent boundaries, authenticated IPC, exact ownership and permission checks. Apply backpressure to complete encoded access units without dropping dependent H.264 frames. Reuse worker-local scaling storage with byte-identical full-display mapping.

**Tech stack:** Rust, DXGI, NVENC/OpenH264, WebRTC, React, and Windows native test sources.

## Evidence and acceptance

- Actual local browser input produced `mrd-local-test` in the dedicated blank Notepad. Paired reconnection accepted video, pointer and keyboard automatically.
- 1080p30 direct session `429cb647-ab5f-45ba-b90e-a91d488d4e62` averaged approximately 29.3 browser total-frame counts/s over the measured window with no increase in dropped/corrupted counts. Agent CPU was about 1% of this 32-logical-processor host; its VideoEncode counter was about 3.2%.
- 1080p60 relay session `49db5cf4-b6b9-4024-bc2b-11fefdfd16fb` showed significant browser drops. Its measured total count was about 44.2/s and non-dropped count about 31.5/s. Agent CPU was 1.7–2.1%, VideoEncode about 6.4%. It did not exit during the measured window.
- 1440p120 direct session `87cb9533-8d1b-4f2a-9e07-a2a5099a1fd4` failed at 18:59:16 UTC: exact Agent event `capture_encoded_unit_rejected` and service `target_media_capture_drain/guard_rejected`. Existing producer treats a full three-unit queue as fatal, alongside validation rejection.
- The old PowerShell WPF source produced only 348 updates in 45.208s. It is unsuitable as a 60 Hz source. The physical display is 60 Hz: requested 120 capture frames/s is not 120 unique display updates/s.
- Browser playback counts are not RTP counts, independent source frames, or end-to-end latency. Direct and relay observations are separate runs, not a controlled route comparison.

## Task 1: Reliable dynamic test source

Create `tests/mobile-gateway/moving-source-fast.ps1` and its compiled native helper. Paint only its own 900x320 window at (20,20), without activating it or sending input, and close after a bounded 45 seconds. Count actual distinct content paints and elapsed time. Verify a 10-second native run before using it as load; do not label a slow source 60 Hz.

## Task 2: Scaling workspace

Modify `apps/mrd-session-agent/src/capture.rs`; isolate equivalence/reuse tests in `capture_profile_frame_tests.rs`. First demonstrate allocations/copies in the original path. Introduce one preparer per capture worker, retaining a target buffer and floor-mapped x/y offsets. Same-size frames are borrowed. Preserve formats, timestamp, malformed-input rejection and full-screen pointer geometry. Compare every output byte with the old algorithm, including non-integer scaling and actual 2560x1440→1920x1080 geometry. Run a finite release benchmark against the original loop; report synthetic CPU scaling cost separately from full product throughput.

## Task 3: Bounded producer backpressure

Modify `apps/mrd-session-agent/src/media.rs` and add `capture_queue.rs`. Distinguish accepted, full-with-owned-unit, and invalid-with-owned-unit outcomes while keeping the original boolean queue API compatible. A producer may wait only on Full, with the mutex released, respecting its stop flag. Keep capacity three, payload bounds, exact session/resource identity and monotonic sequence. Never evict a P-frame, reorder access units or silently accept an invalid unit. Test delayed consumption, preserved order, full-queue cancellation, invalid/replayed/oversized units and missing/poisoned queue behavior. Wire the helper into the capture worker after the scaling change is frozen.

## Task 4: Diagnose browser losses

Only if playback drops remain, add strict numeric inbound-video statistics through the existing browser peer and expanded statistics UI. Expose no raw RTC reports, SSRC, candidate addresses, credentials or arbitrary strings. Stop sampling when collapsed or terminal. Separate receive, decode and playback counters and use valid consecutive samples for deltas.

## Verification and installation

Use the existing active worktree and `--target-dir L:\project\mini-remote-desktop\target --locked --offline`. Run focused regressions, then the Agent library suite once after final integration. Review source and test changes before committing/pushing. Build a pinned release Agent from the clean commit and reuse the established reviewed Windows upgrade route, preserving identity, configuration, keys and audit data. Verify the running Agent hash and process ownership, then repeat browser 30/60/120 runs with the fast source and bounded CPU/GPU observers. End each test through the product UI and retain the dedicated input proof.

### Verified source results before installation

- Compiled WinForms source: 577 distinct paint states in 10.032 seconds; approximately 57.78 updates/s measured between first and last paints. This is not a physical presentation counter.
- Scaler release benchmark: 180 ABBA samples per variant, old median 3963.9 us / p95 4593.8 us; reusable workspace median 2150.4 us / p95 2365.8 us. Approximately 45.75% less median helper time, not an end-to-end FPS result.
- Full-queue legacy publication reproduction: slow consumption and stop/cancellation cases failed before the helper change. Typed queue and producer tests passed after the change.
- Final integrated Agent library: 186 passed, 0 failed, 12 ignored. Scope formatting and diff checks passed. Independent source review found no must-fix in scaler, queue ownership or capture-worker integration.
- The runtime outbound queue remains separately bounded at 32 messages and retains its original failure handling. Producer backpressure does not imply tolerance of arbitrary downstream stalls; actual 120-frame product verification remains necessary.

## Guest create diagnostics

The existing finite rejection logger already leaves admission and masked HTTP errors unchanged. A three-way split of its combined first guard distinguishes absent target, absent access and invalid password after the same existing password-verification work. This diagnostic change does not establish the cause of earlier intermittent HTTP401. Password values in the browser automation's read-only password DOM are redacted; their apparent `<redacted>` length is not evidence of a product input defect.
