# 主线端到端测试设计

**Date:** 2026-06-11

## Goal

设计一套贴合当前主线的端到端测试体系，覆盖从本地媒体链路、UI shell 到
`mrd-service` 服务边界，再到双机 LAN 远程桌面的真实闭环。目标不是把所有带
`e2e` 名字的测试都视为产品级证明，而是建立一套分层证据链：

- 本地组件基线证明采集、编码、传输封装、解码、渲染各环节可用。
- 本地服务边界证明 `Rdesk -> mrd-service -> transport -> renderer` 的主线入口可用。
- 双机 LAN 证明真实 peer、真实 discovery、真实会话、真实帧流可用。
- 故障端测证明断链、降级、清理、错误分类可解释。
- 报告与门槛统一，让 UI 测试工作台、PowerShell 矩阵和 Rust integration tests 使用同一套通过标准。

本文默认“端测”指端到端测试，不包含 `junk/` 下历史实现。

## Current Mainline Entry Points

| Layer | Existing entry | Current value |
| --- | --- | --- |
| Frontend route | `apps/Rdesk/src/app/components/TestWorkbench/E2ETestPage.tsx` | `/test/e2e` 发起 LAN/cross-device 场景，承接手测和 URL autorun。 |
| Frontend orchestration | `apps/Rdesk/src/app/services/lanE2eAutomationService.ts` | 已定义 `LanE2EAutomationReport`、stage、failure reason、cross-device scenario 和 fault plan。 |
| Tauri local harness | `apps/Rdesk/src-tauri/src/test_harness.rs` | 本地 capture/encode/decode/render/transport 可视化链路，适合做单机媒体链路验证。 |
| Tauri test orchestrator | `apps/Rdesk/src-tauri/src/test_orchestrator.rs` | 统一 run/scenario/config/classification 模型，适合收敛本地测试工作台结果。 |
| Service IPC tests | `apps/mrd-service/tests/*` and `crates/mrd-ipc/tests/*` | 验证 mrd-service IPC、web bridge、hard-cut service 行为。 |
| Synthetic integration | `tests/integration/automated_e2e_matrix.rs` | 确认 synthetic capture -> encode -> QUIC AU -> decode -> render 的可复现链路。 |
| Component matrix | `tests/component-matrix/` | 单组件性能边界与硬件能力检测。 |
| Transport benchmarks | `tests/benchmarks/` | local/paired LAN canary、profile matrix、threshold 和 artifact schema。 |

## Test Pyramid

```text
              Cross-device LAN / fault端测
          Service-boundary local dual-process端测
      UI route + IPC contract + local harness端测
  Component matrix / synthetic pipeline / unit contract tests
```

### L0: Contract And Component Baseline

Purpose:

- 捕获接口漂移、命令缺失、硬件能力缺失和单组件性能退化。
- 作为所有端测的 preflight，不直接宣称远程桌面可用。

Required coverage:

- `pnpm test` 覆盖 `lanE2eAutomationService`、`E2ETestPage`、Tauri command contract。
- `cargo test -p mrd-ipc` 覆盖 IPC DTO 和 transport contract。
- `cargo test -p mrd-service` 覆盖服务内部 session/IPC/web bridge 行为。
- `tests/component-matrix` 覆盖 capture、encode、decode、transport sender/receiver、render 边界。

Pass rule:

- 缺硬件时必须返回 `skipped` 或 capability-gated failure，不能以零帧成功。
- contract 测试失败时禁止进入 L1 以上测试。

### L1: Local Synthetic Pipeline

Purpose:

- 用 deterministic/synthetic 输入验证完整媒体链路的算法正确性。
- 避免桌面权限、GPU、LAN discovery 影响基础链路判断。

Entry:

```bash
cargo test --manifest-path tests/integration/Cargo.toml synthetic_capture_encode_transport_decode_render_matrix
```

Required assertions:

- encoded access units > 0。
- transported access units == encoded access units。
- decoded frames > 0。
- rendered frames > 0。
- report 写入 `target/e2e-matrix/automated-e2e-matrix-report.md`。

Limit:

- 该层不是产品端测，只证明 pipeline 逻辑闭环。

### L2: Local UI + Harness端测

Purpose:

- 验证 Rdesk 测试工作台、Tauri command、local harness、native renderer 的同机链路。
- 捕获 UI shell 与 Tauri 本地命令的集成错误。

Entry:

- UI: `/test/e2e` local scenario。
- Tests: `apps/Rdesk` 下 `E2ETestPage.test.tsx`、`RemoteDisplayWindowPage.test.tsx`、contract tests。
- Tauri: `test_harness.rs` and `test_orchestrator.rs` backed commands。

Required scenarios:

| Scenario | Requirement |
| --- | --- |
| `local.harness.capture_only` | capture available, frame counter grows, permission failures are explicit. |
| `local.harness.software_smoke` | synthetic or platform capture + OpenH264/software decode + renderer upload. |
| `local.harness.hardware_smoke` | Windows/NVIDIA path uses DXGI/WinRT + NVENC + NVDEC + D3D11 when capability ready. |
| `local.ui.remote_display_window` | remote display window context created, renderer state reports uploaded/presented frames. |

Pass rule:

- `dataPlaneVerified=true` only when frames come from harness/session data plane, not static UI preview。
- `mediaVerified=true` only when decode/render counters increase。
- display window failure is `failed/display_window_failed`，不能降级成 completed。

### L3: Local Dual-Process Service Boundary端测

Purpose:

- 在一台机器上启动两个独立 `mrd-service` 实例和独立 IPC endpoint，验证主线服务边界。
- 这是进入双机 LAN 前的硬门槛。

Entry:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 `
  -ProfileId 1080p60,2k60,2k144 `
  -DurationSecs 30 `
  -NoBuild
```

Required evidence:

| Metric | Gate |
| --- | --- |
| service instances | two separate process ids and IPC endpoints |
| discovery | controller discovers agent through LAN discovery path |
| session | session id present in runtime snapshot, probe snapshot, and display context |
| frames | decoded frames and render presented frames both increase |
| renderer | native path preferred; WebView/PNG preview is diagnostic only |
| cleanup | both service instances close the session and release display/capture state |

Failure classes:

- `service_unhealthy`
- `peer_not_found`
- `session_start_failed`
- `no_remote_frames`
- `decode_error`
- `render_error`
- `stop_failed`

### L4: Cross-Device LAN端测

Purpose:

- 证明真实两端 Rdesk/mrd-service 在同一 LAN 下能发现、协商、建连、出帧、渲染、停会话。
- 这是当前主线可以承诺的最高产品证据层；不等同 WAN/NAT 产品可用。

Entry:

- UI: `/test/e2e` -> `开始跨设备 E2E`。
- URL autorun:

```text
/test/e2e?autorun=lan-e2e&scenario=cross.e2e.remote_display_smoke&targetDeviceId=<peer-device-id>&transport=quic&timeoutMs=15000&minDecodedFrames=20&minFps=2
```

- Paired canary:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -TargetDeviceId lan-PEER-ID
```

Required scenarios:

| Scenario | Gate |
| --- | --- |
| `cross.e2e.discovery` | peer visible, service build id present, capability summary present. |
| `cross.e2e.remote_display_smoke` | capture source selected, receiver active, display opened, decoded/rendered frames grow. |
| `cross.e2e.media_profile` | requested and selected profile recorded; downgrade is explicit. |
| `cross.e2e.input_control` | key/mouse action sent through control plane and acknowledged by agent. |
| `cross.fault.recovery` | supported fault is injected; unsupported fault returns skipped, never fake success. |

Profile policy:

| Profile | Expected behavior |
| --- | --- |
| `smoke.720p30` | must run on supported desktop platforms, degraded software path allowed. |
| `interactive.1080p60` | hardware preferred; software path must mark degraded. |
| `lan.2k144` | only allowed when both sides advertise capture/encode/decode/render/display refresh capability. |
| `diagnostic.software` | used for isolation, not performance acceptance. |

Pass rule:

- Same branch/build id required for performance comparisons。
- Cross-device FPS comparison only valid when selected profiles match。
- If target display cannot satisfy requested refresh, status is `skipped/display_refresh_limited` or `profile_downgraded`。
- No accepted row may use local harness frames as proof of remote display。

### macOS Controller -> Windows Agent Preparation

Current development host is macOS. The first cross-platform target is:

- controller: macOS Rdesk UI shell, test workbench, receiver/display window, input sender.
- agent: Windows `mrd-service` + Rdesk build with DXGI capture, NVENC/OpenH264 encode,
  QUIC media v3/v2, NVDEC/software decode evidence, and D3D11 native render capability
  advertised through LAN discovery.

Preparation checklist:

| Area | Required before paired run |
| --- | --- |
| build parity | both machines run the same git commit or an explicit expected peer build id. |
| discovery | Windows agent advertises `service_build_id`, `media_protocol_version`, `media_capabilities`, `p2p_control_addr`, and QUIC transports. |
| network | both machines are on the same LAN/VLAN; firewall allows UDP discovery, local service IPC/probe diagnostics, and QUIC media/control ports. |
| Windows agent | screen capture path is available, GPU capability snapshot is current, and native D3D11 render path is not reported as WebView-only. |
| macOS controller | run L0/L1 checks plus macOS local dual-process smoke before pairing; use the UI autorun URL for the cross-device attempt. |
| artifacts | set `MRD_E2E_ARTIFACT_ROOT` on the controller so `summary.json`, `timeline.json`, `metrics.csv`, logs, and frame artifacts are preserved. |

macOS local preflight:

```bash
pnpm --dir apps/Rdesk type-check
pnpm --dir apps/Rdesk test
tests/benchmarks/scripts/run_local_dual_process_lan_canary_macos.sh --profile-id 1080p60 --duration-secs 30
```

macOS controller paired entry:

```text
/test/e2e?autorun=lan-e2e&scenario=cross.e2e.remote_display_smoke&targetDeviceId=<windows-peer-device-id>&transport=quic&timeoutMs=15000&minDecodedFrames=20&minFps=2
```

The current fully automated paired LAN runner is PowerShell-based and remains the Windows
device-lab path. A macOS shell paired runner should be added before treating macOS controller
to Windows agent as a scheduled CI/device-lab lane; until then, macOS->Windows evidence is a
manual L4 run using the same canonical artifacts and gates.

### L5: Fault And Recovery端测

Purpose:

- 验证断链、权限、profile 降级、渲染窗口丢失、服务进程异常后的状态机和报告质量。

Fault matrix:

| Fault | Injection owner | Expected result |
| --- | --- | --- |
| `network.pause_peer` | mrd-service test-only fault command | no-frame state appears, then failed or recovered with timeline. |
| `renderer.detach_surface` | mrd-service/render registry | display failure classified, session cleanup succeeds. |
| `profile.downgrade` | agent capability negotiation | selected profile differs and report marks downgrade. |
| `capture.permission_revoked` | platform-specific capability probe | skipped/failed with permission reason. |
| `process.kill_agent_service` | harness script, not UI | peer lost and cleanup/diagnostics artifacts generated. |

Rule:

- UI 只发送 fault intent；真正 fault injection 在 `mrd-service` 或 harness script 中执行。
- 未实现 fault command 时返回 `skipped/fault_injection_unsupported`。

## Report Model

所有 L2 以上端测报告至少包含：

- `run_id`
- `scenario_id`
- `git_commit`
- `controller` device/build/capability snapshot
- `agent` device/build/capability snapshot when cross-device
- `requested_profile`
- `selected_profile`
- `transport_kind`
- `capture_source`
- `display_mode`
- `stage_events`
- `fault_events`
- `runtime_snapshots`
- `probe_snapshots`
- `media_pipeline_snapshot`
- `metric_series`
- `artifacts`
- `final_status`
- `failure_reason`
- `human_message`

Canonical artifact layout:

```text
artifacts/e2e/<date>/<run_id>/
  summary.json
  timeline.json
  metrics.csv
  controller.log
  agent.log
  artifact_manifest.json
  first-frame.png
  last-frame.png
  failure.txt
```

`first-frame.png` and `last-frame.png` are generated only when the runtime supplies
`first_frame_png_base64` and `last_frame_png_base64`; otherwise `artifact_manifest.json`
marks them as missing optional artifacts rather than creating fake images.

## Acceptance Gates

### Development Gate

Required before merging normal feature work that touches session, IPC, transport, render, or E2E UI:

```bash
cargo test -p mrd-ipc
cargo test -p mrd-session
cargo test -p mrd-application
cargo test -p mrd-service
```

```bash
cd apps/Rdesk
pnpm test -- --run src/app/services/lanE2eAutomationService.test.ts src/app/components/TestWorkbench/E2ETestPage.test.tsx src/app/adapters/tauri/contract.test.ts
pnpm type-check
```

### Mainline E2E Gate

Required before claiming LAN remote-display readiness:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 `
  -ProfileId 1080p60,2k60 `
  -DurationSecs 30 `
  -NoBuild
```

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -TargetDeviceId lan-PEER-ID `
  -ProfileId 1080p60,2k60 `
  -DurationSecs 30
```

Required pass conditions:

- local dual-process and paired LAN both produce structured reports。
- decoded frames > 0 and rendered frames > 0。
- first frame <= 5 seconds for smoke profile。
- no continuous zero-frame window longer than 3 seconds after first frame。
- stop leaves both sessions closed。
- failures use the enumerated `LanE2EFailureReason` style rather than free-text only。

### Performance Gate

Required before comparing high-refresh claims:

- both peers selected the same profile。
- active display refresh satisfies requested FPS。
- hardware path is explicitly present in classification。
- queue depth and render replacement counters are included。
- comparison uses local selected-profile baseline and paired LAN result ratio。

## Implementation Plan

Phase 1: Report normalization.

- Add an `artifacts/e2e/` writer for `LanE2EAutomationReport` or bridge existing benchmark writer into the UI workflow.
- Include `git_commit`, service build id, selected profile, active display mode, and classification in every report.
- Map frontend `LanE2EFailureReason` to benchmark/script failure classes.

Phase 2: Local service-boundary gate.

- Promote local dual-process LAN canary to the standard pre-cross-device command.
- Ensure each run proves two process ids, two IPC endpoints, discovery path, session id, and cleanup.
- Fail if native display creation is requested but only WebView preview produced frames.

Phase 3: Cross-device scenario tightening.

- Require peer build id and capability snapshot before starting performance profiles.
- Implement `cross.e2e.input_control` only after control-plane ACK is available.
- Treat profile mismatch as skipped/degraded unless the scenario explicitly tests downgrade.

Phase 4: Fault command support.

- Add `cross_e2e_inject_fault` in `mrd-service` for `network.pause_peer` and `renderer.detach_surface`.
- Keep process-kill tests in scripts, not UI code.
- Export timeline artifacts that show fault injection, observed state change, recovery/cleanup.

Phase 5: CI/device lab shape.

- CI runs L0/L1 on generic hosts.
- Windows GPU runner runs component matrix and local service-boundary smoke.
- Manual or scheduled device lab runs L4/L5 and publishes artifacts.

Implemented entry:

- `.github/workflows/mainline-e2e.yml`

Workflow shape:

| Job | Runner | Trigger | Scope | Artifact |
| --- | --- | --- | --- | --- |
| `l0-l1-generic` | `ubuntu-latest` | PR, push, manual, schedule | frontend workbench contracts, `mrd-ipc`, `mrd-session`, `mrd-application`, `mrd-service`, Tauri report writer, synthetic pipeline matrix | `l1-synthetic-e2e-report` |
| `windows-gpu-smoke` | `self-hosted`, `Windows`, `X64`, `gpu` | manual with `run_windows_gpu_smoke=true` | component matrix and local dual-process service-boundary smoke | `windows-gpu-mainline-e2e` |
| `device-lab` | `self-hosted`, `Windows`, `X64`, `device-lab` | schedule or manual with `run_device_lab=true` | paired LAN L4 remote-display smoke and optional L5 fault recovery | `device-lab-mainline-e2e` |

Device lab configuration:

- `MRD_DEVICE_LAB_TARGET_DEVICE_ID` repo/org variable may provide the default peer id.
- `MRD_DEVICE_LAB_TARGET_ADDRESS` may provide the peer IPv4 address for diagnostics.
- `MRD_DEVICE_LAB_PROFILE_ID` may override the default `1080p60` profile list.
- `MRD_DEVICE_LAB_DURATION_SECS` may override the default `30` second sample window.
- `MRD_DEVICE_LAB_INCLUDE_FAULT_RECOVERY=false` disables scheduled L5 fault recovery.

The paired LAN script accepts `-ScenarioId cross.e2e.remote_display_smoke` for L4 and
`-ScenarioId cross.fault.recovery` for L5. It forwards the scenario as
`MRD_LAN_E2E_SCENARIO`, and Tauri autorun converts that to the `/test/e2e?scenario=...`
route parameter.

## Non-Goals

- 不把 LAN 端测结果宣传为 WAN/NAT/relay 可用性证明。
- 不从 `junk/` 恢复旧 harness 作为主线证据。
- 不让 UI 直接杀进程、改网络栈或伪造 fault 成功。
- 不接受纯 loopback 或 synthetic pipeline 作为双机远程桌面通过证据。
