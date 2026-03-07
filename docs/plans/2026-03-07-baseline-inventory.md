# Mini Remote Desktop Baseline Inventory

**Date:** 2026-03-07

**Baseline Root:** `G:\Project\mini-remote-desktop`

**Recovery References:**

- `G:\修复\ProjectTest\remote-desktop\mini-remote-desktop`
- `G:\修复\ProjectTest\remote-desktop\mini-remote-desktop\worktrees\layered-core-migration`

## Current Top-Level Inventory

Current baseline roots observed before rebuild:

- `Rdesk`
- `Rdesk-Server`
- `agent-rust`
- `controller-rust`
- `signaling-rs`
- `web`
- `web-client`
- `server`
- `common-control-proto`
- `heartbeat-rs`
- `agent-python`
- `client-qt`
- `tests`
- `tools`
- `docs`

## Rebuild Policy

1. `G:\Project\mini-remote-desktop` is the only writable mainline.
2. Recovery trees are reference-only and must not be copied wholesale.
3. Historical projects will be moved out of the product path during rebuild.
4. Product ownership converges to:
   - `apps/Rdesk`
   - `apps/Rdesk-Server`
   - `apps/realtime-server`
   - `crates/*`
   - `labs/GPUTest`

## First Structural Goal

Add the following new top-level roots before any major move:

- `apps/`
- `crates/`
- `labs/`
- `junk/`

These roots establish the rebuild target and reduce ambiguity for subsequent moves.

