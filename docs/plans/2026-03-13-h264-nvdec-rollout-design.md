# H264 NVDEC Rollout Design

## Goal

Move H264 NVDEC from an experimental capability into a publishable rollout policy that is observable, configurable, and safe to fall back at runtime.

## Current State

- `mrd-decode-nvdec` already provides a working Windows NVDEC H264 decode path with diagnostics.
- The app already exposes NVDEC runtime capability, current decoder selection, and an experimental in-memory toggle.
- The realtime decode path still defaults to `h264_software` unless the user explicitly enables the experimental NVDEC preference.
- The current preference is process-local only and does not survive restart.

## Requirements

- Introduce a formal decoder policy instead of an experimental boolean.
- Persist the policy across app restarts.
- Keep rollout safe: the runtime must automatically fall back to software decode when NVDEC is unavailable or fails.
- Make the active policy and fallback behavior visible in Tauri responses, settings UI, and remote session UI.
- Preserve the current stable default until policy and fallback behavior are fully in place.

## Non-Goals

- Do not make HEVC or Main10 production-ready in this change.
- Do not change the decode interface exposed by `mrd-decode`.
- Do not add new media formats or new transport layers.

## Policy Model

The app will expose a persisted `DecodePolicy` with three values:

- `auto`
- `software`
- `nvdec`

Semantics:

- `software`: use `h264_software` directly and never attempt NVDEC.
- `nvdec`: prefer NVDEC first, but automatically fall back to software if runtime probe, decoder creation, or decode execution fails.
- `auto`: safe production default. In this rollout, `auto` remains conservative and behaves like software-first with NVDEC available for explicit policy selection and future staged promotion.

This gives us a publishable surface immediately without forcing a default NVDEC rollout before more field data exists.

## Persistence

A small local settings file will be added on the Tauri side to persist decoder policy. The file only needs to store rollout-related settings for now, with room to grow later.

The backend will:

- load persisted policy at startup
- default to `auto` when settings are absent or unreadable
- expose commands to read and update the policy
- update the in-memory `WebrtcHost` policy whenever the persisted value changes

## Runtime Selection

`WebrtcHost` remains the runtime decision owner.

At session start, H264 decoder selection will evaluate:

1. persisted policy
2. NVDEC runtime probe result
3. current fallback state

Selection behavior:

- `software`: choose `h264_software`
- `nvdec`: try `nvdec`, then fall back to `h264_software`
- `auto`: choose `h264_software` first in this rollout

The selection result will continue to populate:

- `preferred_decode_backend`
- `active_decode_backend`
- `decode_backend_reason`

Additional runtime state will track:

- `decode_policy`
- `decode_fallback_count`
- `last_decode_fallback_reason`

## Failure Handling

Fallback conditions:

- runtime probe says NVDEC is not healthy enough for the requested path
- `create_decoder("nvdec")` fails
- decode path returns an execution error after selection

Runtime behavior:

- if policy is `software`, no NVDEC attempt occurs
- if policy is `nvdec`, NVDEC failures increment fallback counters and switch the active decoder path to software for the session
- if policy is `auto`, runtime still stays on software-first in this rollout

This keeps session behavior stable and gives operators enough data to decide when `auto` can later be promoted to NVDEC-first.

## API and UI Changes

Tauri:

- replace the boolean preference response with a structured decoder policy response
- add read/write commands for decoder policy

Frontend:

- replace "experimental NVDEC" toggle with a decoder policy selector
- show `auto / software / nvdec`
- continue to show capability summary and per-session preferred/active decoder details
- show fallback count and latest fallback reason where the session snapshot is displayed

## Testing Strategy

Backend tests:

- policy persistence load/save/default behavior
- policy-driven backend order selection
- fallback accounting and snapshot fields
- Tauri helper roundtrip for policy read/write

Frontend tests:

- service invoke mapping for policy read/write
- existing frontend cannot be executed in this environment, so static integration will be kept minimal and localized

Regression:

- `cargo test -p app -- --nocapture`
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`

## Rollout Outcome

After this change:

- H264 NVDEC will have a real product policy surface
- policy will survive restart
- sessions will remain safe because fallback stays automatic
- operators and users will be able to see both capability and actual decode behavior
- the codebase will be ready for a later change that promotes `auto` to NVDEC-first when confidence is high enough
