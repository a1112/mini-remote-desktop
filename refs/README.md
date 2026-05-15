# Reference Projects

`/refs` is a reference-only area for pinned upstream projects and notes. It is not an architecture source of truth for `mini-remote-desktop`.

Allowed use:

- Read upstream architecture, APIs, state machines, benchmark methodology, and platform integration patterns.
- Record concise notes under `refs/notes/`.
- Compare behavior against the pinned tags in `refs/reference-tags.lock.json`.

Disallowed use:

- Do not directly copy GPL or AGPL code into this repository's mainline implementation.
- Do not treat reference project internals as accepted architecture without a corresponding `docs/plans/` decision.
- Do not modify submodule contents as part of normal product work.

Architecture decisions belong in `docs/plans/`. Product entrypoints belong in `apps/`. Reusable Rust implementation belongs in `crates/`.
