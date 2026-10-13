# Agent capture failure diagnostic

The local browser session `c4b10cca-eac1-4bbf-980a-9338bb578427` proved a direct route, then produced service Event 2 `target_media_capture_drain / guard_rejected` at 01:11:29.965 UTC. Its original Agent process 50744 disappeared; the service launched replacement 73456 at 01:11:32.848 UTC. No frame or input has been verified.

The worker currently reports static failure categories only to stderr/tracing. The installed SCM launcher creates the Agent without a console or redirected standard handles, and the Agent has no tracing subscriber. Thus the original worker cause is unavailable. Worker completion can cause `capture_access_units` to return an error, terminate the Agent runtime, and make the service's next target validation fail.

Add a private Agent diagnostic module that writes Windows Application Event 3 under the existing MiniRemoteDesktop source. Its record has exactly four fields: fixed event name, actual process ID, canonical session UUID from the already authorized worker resource or exact access unit, and a finite cause enum. Cover the twelve existing abnormal worker exits, including source-dimension conversion and sequence overflow. Also cover three runtime-pump failures while the exact access-unit session is in hand: rejected session binding, invalid IPC conversion, and unavailable outbound queue. Stop requests suppress worker diagnostics; the synchronous runtime pump processes only resources still owned after prior stop handling. Event sink failure is best effort and cannot alter capture, cleanup, authorization, or exit behavior.

Alternatives considered: redirecting SCM child stderr would change privileged handle inheritance and admit unnecessary text; a new Agent IPC diagnostic message would expand the authenticated protocol. The fixed Windows event requires neither change.

Keep all existing profile, payload, queue, grant, password-generation, desktop, and resource checks. Do not catch native panics or report generic launcher failures using guessed session identity. Desktop resolution and event-context failures before a trusted access-unit session are outside this scoped diagnostic. A missing event therefore does not rule out runtime or native failures. If the next event identifies a specific failure, reproduce that condition and fix its source before measuring performance.

The user has authorized continued automatic local connection tests, updates, and implementation without repeated permission requests. Implementation and review continue in the existing isolated worktree.
