# Manual temporary password refresh

User instruction: change the temporary password refresh frequency to manual.

The resident service retains its current in-memory password until the user explicitly refreshes it. A service restart initializes a new password; no plaintext or reversible password is stored on disk. Explicit disable still erases the secret and revokes applicable authority. Identity/auth changes retain their existing security fences.

Keep the existing maximum ten-minute signed publication lease. Renew that lease without changing password, salt, verifier, or generation. The server accepts a monotonic same-generation renewal only when the authenticated device, pinned key, auth version, enabled state, salt, verifier HMAC, and scopes all match. Reject lower generations, changed same-generation material, stale leases, and attempts to re-enable disabled state. Renewal does not revoke pending requests or extend existing browser/guest authority, grants, credentials, or reservations. Explicit refresh still advances generation and invalidates the old password.

IPC status gains optional `refresh_mode` (`manual` / `automatic`), defaulting to absent when decoding older servers/clients. New resident state reports `manual`; the UI only labels manual refresh when that metadata is present. Existing services retain their expiry display. Manual mode shows stable refresh instructions rather than a password-rotation countdown. Status polling remains operational and never requests rotation.

Alternative considered: simply stop automatic rotation. Rejected because its ten-minute lease would then expire and make remote access unavailable. Persisting a permanent password is outside this request. Bounded publication renewal provides manual refresh with the existing custody and session deadlines.

Verify elapsed-time renewal preserves the same secret/generation, offline gaps and recovery, explicit refresh/disable races and publication acknowledgements, backend replay/material mismatch rejection, and frontend accurate mode display. Actual Air media/input acceptance remains a separate live check; its previous requests expired without local consent.
