# Capture access-unit budget

The local browser session reached authenticated relay connectivity, then the
capture Agent exited with `capture_encoded_unit_oversized`. The service reported
`target_media_capture_drain / guard_rejected`. A native reproduction using the
production DXGI capture, 1920x1080 profile mapping and NVENC factory produced a
298,643-byte periodic keyframe at sequence 31, exceeding the 131,072-byte IPC
access-unit limit.

The Agent now selects dedicated bounded encoder constructors. NVENC retains the
approved bitrate and uses CBR with a single-frame VBV, initial delay and keyframe
scale of one, with temporal AQ disabled for this bounded mode. A typed vendor
setter preserves the ABI and unrelated flags. OpenH264 applies a conservative
bitrate ceiling and verifies the complete normalized Annex B access unit. An
oversized software unit permanently rejects further encoding to prevent sending
dependent frames after a withheld reference frame. Ordinary encoder constructors
retain their existing behavior.

The authenticated IPC limit, profile geometry, scopes and lifecycle checks remain
in place. Rate control is not a mathematical guarantee for every possible scene;
the final complete-unit guard remains authoritative.

Verification on LCX_ACE (RTX 5060 Ti, 2560x1440 desktop):

- Native regression: 61 input frames, 61 complete access units, 61 decoded
  1920x1080 frames, 3 keyframes and 58 interframes. Maximum unit: 42,044 bytes.
- Agent library: 172 passed, 0 failed, 11 ignored.
- NVENC budget arithmetic: 4 passed, 0 failed.
- Software policy and synthetic complete-unit decoding/oversize rejection tests
  passed; the software HD30 sequence decoded every unit.

The native regression verifies capture/encode/decode semantics. Browser display,
remote input, sustained throughput and latency require separate end-to-end runs
after installing the updated Agent.
