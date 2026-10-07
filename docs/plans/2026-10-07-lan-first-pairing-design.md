# Explicit first LAN pairing

The two-device performance run at source `5d4ac8146f1e220f831f1fad6782274b9a386272` is blocked before media: signed discovery admits an untrusted peer, but the production dispatcher always rejects `ApproveTrustedDevice`, the installed UI cannot write machine trust, and only trusted peers can start LAN sessions. Legacy pairing is a separate registry and does not establish signed-key trust.

Implement an explicit installed-UI pairing operation. A candidate is issued only after the existing signature, endpoint/source, namespace, lifetime and replay checks. It binds device ID, Ed25519 key ID and public key, key epoch, and discovery endpoint. Keep its ID stable across continuously valid announcements of the identical binding; replace it after a binding change or expired lease. Approval rechecks the current binding and fresh signed proof under the security gate. Never accept client public-key bytes as authentication evidence.

Only the kernel-verified installed Windows UI in the current interactive logon may approve a candidate. A generic, management or administrator IPC connection cannot use this operation. The UI displays identity and endpoint and requires an explicit click to pair for `screen.view`. Machine pairing saves a permission ceiling, not a session grant. Existing session-agent native attended consent remains mandatory for every incoming session; no input, display changes or unattended policy are enabled by pairing.

Persist the ceiling as authoritative sealed store data and commit it with the initial trust insert and audit. Verify the old sealed v2 store before any v3 migration, migrate atomically, preserve machine identity and audit, and give legacy rows no implicit new permissions. Bind policy into the trust integrity commitment. Suspended or revoked peers cannot be restored by first pairing, existing keys cannot be overwritten, and races must fail closed.

Use the stored ceiling in outgoing and incoming LAN authorization and verified controller-grant installation. Projection-only scope changes are insufficient. Unknown, unsigned, expired, ambiguous, substituted, revoked, audit-unavailable and widened-scope candidates must fail without trust mutation. First-pairing approval must not create a session or replace native consent.

Alternatives considered: enabling generic trust approval would bypass the installed-UI boundary; introducing a second native Agent pairing protocol would add a separate receipt system. The selected UI operation reuses the existing verified actor and signed discovery, while leaving native session authorization intact.

Validation includes cryptographically signed first discovery, candidate renewal/expiry and substitution, persistence/restart and sealed migration/tamper, audit rollback and concurrency, permission-ceiling enforcement, IPC actor rejection and explicit UI confirmation/cancellation. Real two-machine pairing and per-session native prompts require actual user clicks. Do not label waiting for those clicks as a performance failure.

No commit or push is authorized for this patch. Keep the diff and test evidence available for synchronization to the other device. Both machines must build the same reviewed patch and record the base SHA plus patch digest rather than claiming an unchanged clean source SHA.
