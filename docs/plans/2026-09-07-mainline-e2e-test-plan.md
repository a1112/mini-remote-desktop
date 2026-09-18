# 最新主线端到端测试执行计划

日期：2026-09-07。本文中的“端测”指端到端测试。本轮交付是同步主线和制定执行计划，尚未运行完整产品端测。

## 基线与本地改动

- `main` 已从 `a77e1c3` 快进 523 个提交至 `9d7e5987b83759db65eb815182eb83e9daf9b116`，与本次 fetch 的 `origin/main` 一致。
- SSH fetch 因 publickey 认证失败，已通过同一仓库的 HTTPS 地址完成 fetch，origin 地址保持原样。
- 更新前的暂存区、工作区和未跟踪文件保存在 stash `61dd41c66b345cf57241cbc803e71fd5ed337edf`，描述为 `codex-preserve-before-main-update-2026-09-07`。未自动重放到新主线，避免把旧性能实验混入端测基线。原先被忽略的本地产物保留原位。
- Windows checkout 新增可执行 shell 文件后产生了两个纯文件模式差异，已设置仓库本地 `core.filemode=false`；未修改脚本内容。
- 如需继续原实验，优先在旧基线的独立 worktree 使用 `git stash apply --index 61dd41c66b345cf57241cbc803e71fd5ed337edf`。不要在性能基线中直接 pop 旧改动；保留 stash 直到恢复结果确认。

## 本机预检与缺口

| 项目 | 当前核查结果 | 开测要求 |
| --- | --- | --- |
| GPU | NVIDIA GeForce RTX 3070 Laptop GPU，驱动 596.21 | 运行时再次验证 NVENC/NVDEC 能力；HEVC/AV1 按实际能力分流 |
| 显示 | WMI 报告 2560×1600@165Hz；存在虚拟显示适配器 | 记录实际选中屏幕、采集源和 active display mode，不能仅按 WMI 判高刷通过 |
| Rust | 默认 Cargo 1.92.0；另装 1.94.1、1.97.1；CI 固定 1.98.0 | 安装/选择 1.98.0，记录版本；当前 shell PATH 缺少 cargo |
| 前端 | pnpm/node 可调用，`apps/Rdesk/node_modules` 不存在 | 按 lockfile 安装依赖，构建前端 |
| Windows PowerShell | 可通过系统绝对路径启动；当前 PATH 找不到 powershell | 在测试进程 PATH 补齐系统目录 |
| 数据库 | `MRD_TEST_DATABASE_URL` 未配置 | 准备隔离 PostgreSQL 测试库；未跑数据库行不得算通过 |
| 双机 LAN | 尚未选择/核实第二台设备 | 两端同 commit、同构建配置、明确 device ID，完成真实授权 |
| WAN lab | relay/initial-WAN control 和 attestation 变量均未配置 | 配好服务端、两端服务、TURN 与实验控制器后才能运行真实 WAN 行 |

本轮轻量验证：`test_transport_matrix_common.ps1` 退出 0；`test_multi_region_relay.ps1` 输出 contracts passed 并退出 0。`test_paired_lan_canary_common.ps1` 在调用 cargo 时因 PATH 缺失中止，未获得完整通过结论。以上脚本检查不代表真实媒体或远程控制通过。

## 执行顺序与放行条件

| 阶段 | 范围与顺序 | 放行标准 |
| --- | --- | --- |
| P0 环境与合同 | 工具链、锁定依赖、Rust IPC/session/application/identity、服务授权、前端工作台合同、质量门禁 | 必选用例全部通过；构建失败先修复再进入真实端测 |
| P1 单机媒体 | synthetic 矩阵；DXGI→NVENC H.264→QUIC→NVDEC→D3D11 本地基线 | 编码、传输、解码、真实 present 计数增长；记录实际后端和软件降级 |
| P2 本机双进程 | 两个 mrd-service、独立 IPC；1080p60 冒烟→2k60→1080p144→2k144→1600p165 | discovery/session/media/render/cleanup 全闭环，两个实例不能串会话 |
| P3 双机 LAN | Windows↔Windows，先 discovery，再授权远程显示、键鼠输入和停止；随后 Windows↔Linux/macOS | 原生可见首帧、真实画面更新、输入在目标端生效；授权、审计、清理证据齐全 |
| P4 故障与回归 | 丢包/延迟/抖动/MTU；peer 停止、UI 重启、显示关闭、权限撤销、重复建连 | 不崩溃；错误分类明确；旧会话/按键/渲染资源释放；新会话能正常启动 |
| P5 WAN | 有人值守初始授权→TURN UDP/TCP/TLS→键鼠→主中继失效/迁移→关闭 | 真实双方服务和中继证据；路由、generation、控制权限、容量释放一致 |
| P6 稳定性 | 先 180 秒，再 30 分钟、2 小时，最后 8 小时 | 记录帧停顿、恢复、内存/句柄趋势和清理；发现问题保存最小复现后停止扩展 |

先完成 P0–P2，随后执行选定设备的 P3–P4。P5 依赖独立实验环境；不使用单机 coturn 或 fixture 的通过结果替代 WAN 验收。高刷和新编码格式在 H.264 基线通过后再扩展。

## 命令清单

以下从仓库根目录的 PowerShell 执行，每条检查退出码，失败即停止当前阶段。多值参数用 PowerShell 数组直接调用脚本，避免 `powershell.exe -File` 把逗号列表误当单个字符串。首次构建后才允许后续 `-NoBuild`，且需确认二进制属于本次 commit。

### P0：准备与合同

```powershell
$env:PATH = "C:\Users\Administrator\.cargo\bin;C:\Windows\System32;C:\Windows\System32\WindowsPowerShell\v1.0;" + $env:PATH
rustup toolchain install 1.98.0 --profile minimal --component rustfmt --component clippy
$env:RUSTUP_TOOLCHAIN = '1.98.0'
cargo --version
pnpm --dir apps/Rdesk install --frozen-lockfile
pnpm --dir apps/Rdesk type-check
pnpm --dir apps/Rdesk exec vitest run src/app/services/lanE2eAutomationService.test.ts src/app/services/lanE2eTelemetryService.test.ts src/app/components/TestWorkbench/E2ETestPage.test.tsx src/app/adapters/tauri/contract.test.ts src/app/adapters/tauri/commands.controlInput.test.ts
pnpm --dir apps/Rdesk build
cargo fmt --all -- --check
cargo test --locked -p mrd-ipc -p mrd-session -p mrd-application -p mrd-identity -p mrd-quality-gate
cargo test --locked -p mrd-service -- --test-threads=1
cargo test --locked -p mrd-session-agent
cargo test --locked -p mrd-transport-webrtc -- --test-threads=1
& ./tests/benchmarks/scripts/test_transport_matrix_common.ps1
& ./tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
& ./tests/benchmarks/scripts/test_multi_region_relay.ps1
& ./tests/benchmarks/scripts/run_secure_lan_negative.ps1 -OutputDir target/e2e-20260907/security-negative
```

后端按 `.github/workflows/rust.yml` 在 `apps/Rdesk-Server` 的隔离 Python 环境安装 `requirements-dev.txt` 并运行 `python -m pytest -q`。记录 PostgreSQL 实际执行/跳过数量，不把内存 stub 的结果计为数据库验收。测试库和后端凭据使用环境配置，产物中仅记录是否配置。

### P1–P2：本机链路

```powershell
cargo test --locked --manifest-path tests/integration/Cargo.toml --test automated_e2e_matrix synthetic_capture_encode_transport_decode_render_matrix -- --nocapture
cargo test --locked --manifest-path tests/integration/Cargo.toml --test service_agent_media -- --test-threads=1
& ./tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId @('1080p60') -DurationSecs 30 -DisplayModePolicy none -OutputDir target/e2e-20260907/local-smoke
& ./tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId @('2k60','1080p144','2k144','1600p165') -DurationSecs 30 -DisplayModePolicy temporary -NoBuild -OutputDir target/e2e-20260907/local-profiles
& ./tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId @('1080p144') -DurationSecs 30 -NoBuild -LossPct 1 -BaseDelayMs 2 -JitterMs 3 -MtuBytes 1200 -OutputDir target/e2e-20260907/local-impairment
```

单机 benchmark 用现有 `run_paired_lan_canary.ps1 -SkipCross` 建立对应 selected-profile 的组件性能基线；同机双进程报告单独保留，避免混淆开销。显示模式临时切换前记录原模式，结束后核对恢复。

### P3–P4：真实双机

设置 `$peerDeviceId` 为已验证测试设备的真实 ID，并在双方完成授权。为输入用例准备专用空白文本窗口，检查按下/释放、点击、滚轮、坐标映射、窗口失焦以及断链后的按键释放。

```powershell
& ./tests/benchmarks/scripts/run_paired_lan_canary.ps1 -TargetDeviceId $peerDeviceId -ScenarioId @('cross.e2e.discovery','cross.e2e.remote_display_smoke','cross.e2e.secure_remote_display','cross.e2e.input_control') -ProfileId @('1080p60') -DurationSecs 30 -OutputDir target/e2e-20260907/paired-smoke
& ./tests/benchmarks/scripts/run_paired_lan_canary.ps1 -TargetDeviceId $peerDeviceId -ScenarioId @('cross.e2e.media_profile') -ProfileId @('2k60','1080p144','2k144','1600p165') -DurationSecs 30 -RatioThreshold 0.8 -OutputDir target/e2e-20260907/paired-profiles
& ./tests/benchmarks/scripts/run_paired_lan_canary.ps1 -TargetDeviceId $peerDeviceId -ScenarioId @('cross.fault.recovery') -ProfileId @('1080p60') -DurationSecs 30 -OutputDir target/e2e-20260907/paired-fault
```

故障能力须在运行时确认，缺 fault injection 的行标为 unsupported/skipped，不计入必选恢复验收。手工终止测试进程仅针对本次 run 的 PID。每种可用故障独立执行，先验证错误与清理，再重新建连。跨平台先跑 720p30/1080p60 功能闭环，硬件/显示不满足的性能行单列。

### P5：WAN

实验拓扑和变量遵循 `docs/release/multi-region-turn-relay-acceptance.md`。初始 WAN 需要 `MRD_INITIAL_WAN_LAB_CONTROL`、`MRD_INITIAL_WAN_ATTESTATION_PUBLIC_KEY`、`MRD_INITIAL_WAN_ATTESTATION_KEY_ID`；多区域需要 `MRD_RELAY_LAB_CONTROL`。当前均未配置。

```powershell
& ./tests/benchmarks/scripts/run_multi_region_relay.ps1 -Scenario initial_wan_local -OutputRoot target/e2e-20260907/initial-wan
& ./tests/benchmarks/scripts/run_multi_region_relay.ps1 -Scenario all -OutputRoot target/e2e-20260907/multi-region
```

重点复查最新变更涉及的授权入口、WAN WebRTC 媒体/输入、每会话带宽预留、Relay 清理和 WebRTC teardown 时序。覆盖拒绝/超时/重放/撤销、容量耗尽、授权前后服务端断开、TCP/TLS 回退、主中继故障和最终 ReleaseAll。fixture、普通 Rust 测试及 attestation 合同测试独立统计。

## 验收门槛与报告

- 以真实采集、decode 和 present 的增长证明媒体闭环；CPU/PNG 预览、仅元数据以及零帧不能判产品通过。
- 当前 `windows-1080p60-direct.v1.json` 要求可见首帧不超过 4000ms。只有场景和 schema 对应的真实产物才交给该策略评估。
- `windows-secure-lan.v1.json` 要求可信身份、明确授权依据、scope 允许、QUIC 对端认证、至少一帧真实 present、至少一次已认证输入注入、审计事件和完整清理。
- 初始性能目标沿用现有严格计划：1080p60 解码 FPS ≥55，2k60 ≥45，1080p144/2k144 ≥115，1600p165 ≥132；高刷行须实际采集模式满足要求。双机同 selected profile FPS ≥对应本地基线的 80%。这些是本轮验收目标，不是已测结果，也不代表当前 JSON 已自动检查所有指标。
- 记录 capture/encode/send/reassemble/decode/render p50/p95、队列、丢帧、jank、首帧、输入反馈延迟、故障恢复耗时、资源趋势。沿用严格 LAN 参考门槛：receiver.record p95 ≤2ms、render_present p95 ≤3ms、稳态队列 ≤1；网络注入须验证丢弃/延迟计数实际增长。
- 硬件缺失、权限缺失、显示刷新率不足、降级和实验设施缺失必须明确区分。必选行缺证据不得通过；基础设施缺失记 INFRA_FAIL。清理失败、未授权输入、错误画面、进程崩溃为阻断项。
- 每个 run 保存 commit、构建配置、dirty 状态、双方系统/GPU/驱动、device/session/run ID、requested/selected profile、实际 codec/renderer/route、summary、时间线、指标和双方日志。路径统一放在 `target/e2e-20260907/` 分阶段目录，重复运行另加 run ID，避免覆盖。
- 完整回归在对应 Windows/Linux/macOS 环境完成；发布前另执行 workspace Clippy/测试、完整前端测试和后端数据库测试。先前文档中的 PASS 不能沿用为本提交的通过结论。

## 参考入口

- `.github/workflows/mainline-e2e.yml`、`.github/workflows/rust.yml`
- `docs/plans/2026-06-11-mainline-end-to-end-test-design.md`
- `docs/plans/2026-05-17-e2e-transport-strict-test-and-review.md`
- `docs/plans/2026-08-30-core-production-remediation.md`
- `docs/release/multi-region-turn-relay-acceptance.md`
- `tests/quality-gates/policies/windows-1080p60-direct.v1.json`
- `tests/quality-gates/policies/windows-secure-lan.v1.json`
