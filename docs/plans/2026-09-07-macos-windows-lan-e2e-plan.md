# macOS ↔ Windows 局域网端测计划

日期：2026-09-07。范围：用户确认的 macOS、Windows 双机局域网测试。
本文件为执行计划；真实双机端测尚未开始。

## 1. 代码基线与已查证状态

- `main` 从 `3f265b6a` 快进 36 个提交到 `9d7e5987b83759db65eb815182eb83e9daf9b116`，与本次 fetch 的 `origin/main` 一致。
- 原有 `junk/agent-python/cpp_capture/build/` 下 4 个文件删除及未跟踪的 `apps/Rdesk/.env.ios.local` 保留；同步前后核对了工作区状态和本地环境文件内容摘要。
- 本机：macOS 26.6.2、arm64；Rust 1.97.1、Node 26.4.0、pnpm 11.19.0。CI 使用 Rust 1.98.0、Node 22、pnpm 10.33.0；正式构建前对齐项目工具链，避免改动全局默认版本。
- 本工作区未发现 `apps/Rdesk/node_modules`、`target/debug/mrd-service`、`target/release/mrd-service`。需要安装依赖并重新构建，不能复用来源不明的二进制。
- 已通过 macOS runner 合同检查：`bash tests/benchmarks/scripts/test_local_realistic_lan_e2e_macos.sh`；已通过 `git diff --check`。这些不代表媒体链路或双机验收通过。
- 同一 SHA 的 [Mainline E2E 运行 33991501124](https://github.com/lcxinc/mini-remote-desktop/actions/runs/33991501124)：L0/L1 generic host 成功；Gate 0 失败；Device lab L4/L5 取消；secure LAN 正向门禁跳过。查询时另有排队运行，不能作为已通过证据。
- Gate 0 日志为前端 688 passed / 1 failed：`RemoteDisplayWindowPage.test.tsx` 中 `releases active remote keyboard input when the window loses focus`。需先复现和定位，尚不能判断是产品缺陷还是测试时序问题。

## 2. 双机准备

首轮由 macOS 控制 Windows；通过后再测 Windows 控制 macOS。两方向分别出结论。

开测时补齐以下信息：

| 项目 | macOS | Windows |
| --- | --- | --- |
| 系统与架构 | 26.6.2 / arm64，执行时复核 | 待采集 |
| LAN 地址、device ID | 待采集 | 待提供或发现 |
| 网卡、连接方式、网段 | 记录有线/Wi-Fi、VPN及路由 | 同左 |
| 构建 SHA、服务 build ID | 同一固定 SHA | 同一固定 SHA |
| GPU、驱动、编解码能力 | 采集 VideoToolbox 能力 | 采集 DXGI、NVENC/OpenH264 等实际能力 |
| 显示器、分辨率、刷新率、缩放 | 实际采集 | 实际采集 |
| 权限与会话 | 屏幕录制、辅助功能/输入权限；解锁 | 交互桌面、服务与 session-agent 状态；解锁 |

两端使用同一 LAN/VLAN并保持唤醒；先确认 discovery UDP 21116 和实际公布的 QUIC/control 地址可达。按实际配置放行防火墙，IPC 保持本机边界。需要安全身份/配对时使用真实授权配置，记录拒绝原因。

## 3. 执行顺序与通过条件

| 阶段 | 执行内容 | 通过条件 |
| --- | --- | --- |
| A：基线门禁 | 复现失焦键盘释放失败；前端合同、类型检查、构建；IPC/session/service 测试 | 相关合同全通过，失败已定位并解决；若修复代码则更新双方固定 SHA |
| B：本机链路 | Mac 双服务真实 capture→encode→QUIC→decode→native render；Windows 组件能力检查 | 独立服务与 IPC、真实解码和呈现帧增长、会话清理完整 |
| C：Mac→Windows | 正常设备页发起 LAN 会话、被控端授权、首帧、输入、结束；再运行测试工作台跨设备场景 | 双端 session/build/peer 一致；画面确为远端动态桌面；真实输入生效 |
| D：Windows→Mac | 重复 C，单独检查 Mac 采集及输入权限、坐标和缩放 | 独立判定；不以另一方向结果代替 |
| E：性能与稳定性 | 1080p60/H.264 每方向 30 秒冒烟→5 分钟采样→30 分钟持续会话 | requested/selected profile、实际编解码与渲染后端完整；达到门槛，无无解释卡死/泄漏 |
| F：故障与清理 | 拒绝授权、断网恢复、失焦、关闭窗口、服务重启、重复连接/断开 | UI 状态真实，输入释放，无会话/媒体任务残留；恢复或明确失败并能重新连接 |

阶段 A 未通过前不进入正式媒体/双机验收。阶段 B 的本机双进程只能计为本机证据。测试工作台通过后仍需覆盖正常产品入口。

## 4. 现有执行入口

以下命令除第 1 节已列明的检查外均待执行。默认从仓库根目录运行，先完成工具链和依赖准备。

### A：合同与构建

```bash
pnpm --dir apps/Rdesk install --frozen-lockfile
pnpm --dir apps/Rdesk exec vitest run src/app/components/RemoteDisplayWindowPage.test.tsx
pnpm --dir apps/Rdesk exec vitest run
pnpm --dir apps/Rdesk type-check
pnpm --dir apps/Rdesk build
cargo test --locked -p mrd-ipc -p mrd-session -p mrd-application -p mrd-quality-gate
cargo test --locked -p mrd-service -- --test-threads=1
cargo test --locked --manifest-path tests/integration/Cargo.toml --test automated_e2e_matrix synthetic_capture_encode_transport_decode_render_matrix -- --nocapture
```

失焦测试需同时核对单文件和完整套件表现。保留第一次失败日志，不能仅靠重跑覆盖。原生客户端通过 `pnpm --dir apps/Rdesk tauri:dev` 构建启动；部署端须核对实际运行服务与构建产物一致。

### B：Mac 本机真实链路

```bash
bash tests/benchmarks/scripts/run_local_realistic_lan_e2e_macos.sh \
  --output-dir artifacts/e2e/2026-09-07-lan/local-macos \
  --profile-id 1080p60 --duration-secs 30 --codec h264 \
  --scenarios clean,lan,stress
```

该 runner 的 decoded/rendered FPS 门槛分别为目标帧率的 95% / 90% / 80%；网络损伤由发送端注入。记录实际分辨率，该 runner 允许保持源宽高比调整，不能把调整后的 profile 作为严格 1920×1080 的性能结果。

### C：Mac 控制 Windows

使用原生 Rdesk 的 `/test/e2e`，选择 Windows peer、QUIC/LAN，依次运行 discovery、remote display、media profile、input control 与 fault recovery 场景。现有入口示例：

```text
/test/e2e?autorun=lan-e2e&scenario=cross.e2e.remote_display_smoke&targetDeviceId=<Windows实际device-id>&transport=quic&timeoutMs=15000&minDecodedFrames=20&minFps=2
```

该 URL 的 20 帧/2 FPS 只是连通冒烟下限，不能用于 1080p60 性能验收。当前配对自动化脚本是 Windows PowerShell 路径；Mac→Windows 首轮使用原生 UI 和工作台采集证据，不将 Mac 本机双服务脚本当作双机脚本。

### D：Windows 控制 Mac

在 Windows 仓库执行，参数替换为实际 Mac 信息；每个新运行使用独立输出目录：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 `
  -SkipLocal `
  -TargetDeviceId "<Mac实际device-id>" `
  -TargetAddress "<Mac局域网IPv4>" `
  -ScenarioId cross.e2e.remote_display_smoke `
  -ProfileId 1080p60 -DurationSecs 30 -Codec h264 `
  -OutputDir artifacts/e2e/2026-09-07-lan/windows-to-macos-smoke
```

后续以 `cross.e2e.secure_remote_display`、`cross.fault.recovery` 分别运行授权与故障场景。GPU/高刷扩展仅在能力满足时执行。

## 5. 双机重点用例和拟定验收门槛

- 连接：设备发现、授权允许/拒绝、首帧、主动结束、再次连接。请求已受理不能显示为已出画面。
- 输入：移动、单击/双击、拖拽、滚轮、文字、组合键；按住键后切出窗口，确认目标端释放且不持续重复输入。覆盖正常结束和异常断开后的按键释放。
- 显示：移动窗口/播放动态内容证明新帧；缩放、窗口尺寸变化、多屏选择与屏幕边缘坐标；能力不足时明确降级。
- 拟定 LAN 性能门槛：1080p60 同档位稳态 decoded/rendered FPS 各≥54，记录首帧、FPS、RTT、阶段 p95、丢帧、重复帧、CPU/内存。指标采集缺失记为缺证据。54 FPS 是本轮计划门槛，不声称现有 paired runner 默认已强制此值。
- 首帧≤4秒作为本轮双向目标；仓库现有 `windows-1080p60-direct.v1.json` 仅对 `windows.1080p60.direct` 行规定此上限，Mac 方向需独立记录和判断。
- 30分钟稳定性与10次连接/断开作为本轮拟定验收；异常网络/服务重启单独运行，避免污染稳态性能样本。
- 输入成功须同时有发送/接收或注入证据和目标端可见效果；IPC ACK不能单独证明物理输入生效。
- 故障恢复记录具体耗时与结果；尚未约定统一恢复时限，不从“重新连接成功”推断无缝恢复。

## 6. 证据与交付

每个方向、场景与重跑分别使用 run ID，归档到 `artifacts/e2e/<date>-lan/<run-id>/`。保留失败原件。

最低证据：双端 SHA/build ID、能力快照、网络与 profile、peer/session ID、双端日志、时间线、解码/呈现指标、原生画面截图或录屏、目标端输入效果、会话结束后的资源状态。报告区分 PASS、FAIL、SKIPPED、INFRA_FAIL；硬件或权限不满足不得以零帧计 PASS。

本轮下一步：准备 Windows 的 LAN IPv4/device ID、系统/GPU信息和可操作方式；复现 Gate 0 失焦输入失败，完成本机准备后启动 Mac→Windows 首轮。两端真实产品验收结果待执行后单独生成报告。
