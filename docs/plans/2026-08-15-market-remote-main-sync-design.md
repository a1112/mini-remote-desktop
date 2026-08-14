# Market Remote Capability Main Sync Design

## Objective

Integrate the current `main` branch into the remote-capability alignment work
without modifying the user's dirty primary worktree, then publish a reviewable
branch whose merge result has been tested as a coherent repository snapshot.

## Branch Strategy

Create `codex/market-remote-capability-alignment-main-sync` from `ba528b63` in
an ignored project-local worktree. Merge `origin/main` with a merge commit
rather than rebasing 114 feature commits. This preserves the existing history
and confines conflict resolution to one integration point.

## Conflict Resolution Policy

Resolve conflicts by behavior, not by selecting one side wholesale:

- retain `main` security, configuration, macOS, and dependency updates;
- retain the feature branch's authenticated session, transport, observability,
  remote display, and local performance behavior;
- merge public types and protocol contracts additively where both sides added
  independent fields or variants;
- preserve tests from both sides and update expectations only when the merged
  contract intentionally changes;
- avoid unrelated refactors while resolving the merge.

## Verification

The merged tree must have no conflict markers and must pass:

- `cargo fmt --all -- --check`;
- Rust workspace tests, or a documented narrower diagnosis for environmental
  hardware-only failures;
- Rdesk type checking, unit tests, and production build;
- Rdesk Server Python tests;
- transport-matrix and local-performance-suite PowerShell contract tests;
- a final Git diff check and remote branch/PR status audit.

## Publication

Commit the resolved merge on the isolated branch, push it to `origin`, and
create a draft pull request targeting `main`. Do not alter, reset, stage, or
clean the user's primary worktree.
