# Browser consent timeout implementation plan

> Execution follows the existing approved browser remote control design and its expiry/cleanup requirements. Use test-driven development and independent review in this session.

**Goal:** End a browser's pending consent wait at the deadline of its own signed connection intent, rather than waiting for the longer browser identity lease.

**Architecture:** Keep the current attended-only authorization flow and exact signed grant/backend policy checks. Add a local pending consent timer using `session_intent_v3.payload.claims.expires_at_ms`; stop it when a fully verified and backend-approved grant is accepted, or during normal close. Never extend HTTP credentials, signing credentials, password leases, scopes, or target consent.

**Tech stack:** TypeScript, Vitest, existing BrowserRemotePeer and signed protocol fixtures.

## Evidence and authorized scope

The approved `2026-10-09-browser-remote-control-design.md` requires visible expiration and cancellation of stale callbacks. Real session `26a7fd5a-a7a7-4a93-b8fd-8263a644be1c` expired at the target intent deadline `1791693925690`, with no grant, while the browser continued waiting until its later identity deadline `1791693984801`. Source inspection shows a bootstrap expiry timer, but no timer for the signed intent. Mac uses that intent's expiry as the consent deadline. Backend requested status does not independently expire on target-local consent timeout.

## Task 1: Regression tests

Modify `apps/Rdesk/src/app/services/browserRemotePeer.test.ts` only. Use its existing fake clock, real signing fixtures, and socket test adapter. Assert that an ungranted intent fails at its own signed expiry even while bootstrap remains valid and the backend still returns requested. Verify no media/input grant, local socket/peer cleanup, backend close while its credential is valid, no late grant resurrection, and cancellation of the consent timer after a properly verified grant or explicit close. Include the shorter credential-bound deadline and clock advancement boundary as appropriate.

Run `pnpm exec vitest run src/app/services/browserRemotePeer.test.ts` from `apps/Rdesk`. First observe the new timeout behavior test failing because the peer stays waiting; save the red result in ignored verification artifacts.

## Task 2: Minimal correction

Modify `apps/Rdesk/src/app/services/browserRemotePeer.ts` only. Schedule a tracked pending-consent timer immediately after sending and storing the signed intent. Use its actual claims expiry, not a fresh `now + 60_000` or renewed bootstrap. If still ungranted and open at expiry, fail with a specific remote-confirmation timeout and use the existing bounded close/abort cleanup. Clear the tracked timer only after the exact signed grant has passed backend policy validation, and on close. Do not reset the timer on polls/heartbeats. Preserve rejection and closed-state validation, late callback fences, and the existing close failure warning.

## Task 3: Verification and review

Run the peer, service, page, control, protocol and relay-directory tests, `pnpm type-check`, a production browser build with base `/apps/rdesk/`, and `git diff --check`. Review timer/grant races, expired intents, delayed crypto callbacks, cancellation, and safe error text independently. No native service/client rebuild or API change is needed.

## Task 4: Integrate and deploy

Commit the reviewed frontend fix with this plan, integrate into main without touching unrelated untracked files, and push using the existing authorized Git credential workflow. Reuse the bounded web release deployment process with a fresh commit-bound release; retain rollback release and preserve API/realtime/config/identity. Verify served asset hashes and load the new web build. Keep the Mac native candidate on verified `29c151b2` and continue actual consent, video and input testing only after user readiness and real target confirmation.
