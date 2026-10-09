# Temporary password and guest remote access

The user requested temporary passwords and remote control of devices without account login, extending the approved independent browser controller. The initial policy remains attended: a correct temporary password authorizes a session request, and the target UI confirms its scopes. The optional alternative of automatic password approval has not been selected.

## Product behavior

- The installed resident service registers its durable machine identity and displays its existing device code without requiring an account or changing that identity.
- The service generates an eight-character random temporary password, excluding ambiguous characters, with a ten-minute lifetime. The native UI can hide, reveal, copy, refresh, or disable it.
- A web guest enters the target code and temporary password. The browser has no dependency on a controller-side resident service and keeps its temporary identity and scoped credentials in memory.
- The target may be unbound. Account login remains available for device management and existing account sessions.
- Successful password validation leads to the normal target confirmation and signed authorization. Screen and input require their individual approved scopes and real media/channel readiness.
- Refresh invalidates the old password and pending requests. Existing approved grants retain their original bounded lifetime. Disabling temporary access revokes guest sessions and stops local media/input.
- A password is usable only after publication succeeds. Offline signaling, failed publication, expiry, and missing system permissions have distinct honest states. The browser does not show a local `mrd-service offline` banner.

## Authority and storage

Guest access uses an explicit temporary-access authority, separate from account ownership. It does not create a fake user or loosen existing physical-device and account authorization rules. Database constraints make guest and account bindings mutually exclusive.

The server stores only a salted password verifier and its target machine key, device authentication version, generation, scope ceiling, expiry, and enabled state. Publication and changes require the physical machine credential and a context-bound machine signature. Password attempts receive uniform failures and persistent limits shared across API workers.

Guest HTTP and signaling credentials have distinct purposes and are bound to one controller public key, session, target, scopes, authority generation, and expiry. They cannot be used for enrollment, device management, another target, or an expanded scope. Trusted target and server pins, signed negotiation, relay directory verification, and ongoing target authority checks remain mandatory.

Plaintext temporary passwords exist only in resident-service memory and the authorized local UI. They do not appear in ordinary snapshots, events, Debug output, logs, URLs, or persisted browser state.

## Implementation sequence

1. Define backend/native contracts and explicit guest authority schema; retain account APIs and migrations.
2. Implement password publication, verification, rate limits, guest create/inspect/close/relay access, and device-side approval/revocation with cross-await revalidation.
3. Implement resident password lifecycle, protected IPC secret reads and controls, Tauri command forwarding, and strict guest signaling credentials.
4. Add native password controls and a browser guest connection form; reuse the independent browser media/control engine with memory-only guest credentials.
5. Independently review both authorities and run SQLite, real PostgreSQL, Rust protocol/lifecycle, frontend, and actual browser checks.
6. Commit and push the verified changes, deploy a pinned release with rollback backups, update native targets, and accept a real unbound Mac screen/input session.

## Required acceptance

Tests cover guest access to an unbound physical target; target confirmation; wrong, expired, and refreshed passwords; refresh/approval races; cross-worker limits; scope and key substitution; revocation during network awaits; inability to obtain physical credentials; account-session compatibility; and absence of secret material in logs and ordinary state.

Real-device acceptance records the same session ID on the browser and Mac, target capture/encoding and browser decoded-frame increments over thirty seconds, selected ICE route, approved input acknowledgment and a visible non-destructive input result, then session cleanup. Generated video and empty-page browser smoke do not count as real-device acceptance.
