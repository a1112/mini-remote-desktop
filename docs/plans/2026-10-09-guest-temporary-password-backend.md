# Guest temporary password backend implementation plan

**Goal:** Both remote target and browser controller work without logging in, with a short lived machine-published password and local attended approval.

**Architecture:** Physical DeviceJWT plus pinned machine Ed25519 publish salted PBKDF2 verifier. Guest authority is a separate database CHECK domain and separate HTTP/signaling JWT type; account ownership rules remain unchanged. Rotation invalidates pending requests, approved sessions retain their original TTL; disabling revokes every guest grant.

**Tech stack:** FastAPI, SQLAlchemy, PostgreSQL/SQLite test fixture, Ed25519, PBKDF2-HMAC-SHA256.

1. Write and run failing real HTTP tests for signed publication, no-user guest create, wrong code/password uniform errors and rotation.
2. Add explicit DeviceTemporaryAccess and persisted attempt rows; exact signature/TTL/generation checks and constant-time password comparison.
3. Add SessionRequest/BrowserController mutually exclusive account and guest authority with idempotent migrations preserving existing data.
4. Implement isolated guest HTTP credential and guest signaling credential, with trusted target pin retrieval and lock/authority revalidation after awaits.
5. Extend native target inspect/approval and relay access only for the exact validated guest session; enforce local confirmation, scopes and bounded TTL.
6. Run full backend regression, targeted replay/race/limit/schema tests and provide source manifest. Root handles commit, deployment and native acceptance.
