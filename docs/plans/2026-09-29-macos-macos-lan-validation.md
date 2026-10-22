# Mac ↔ Mac 局域网：键鼠与系统声音实现及验收

日期：2026-09-29。仓库：mini-remote-desktop。

本轮按用户要求参考 R-ShareMouse，优先实现鼠标、键盘和系统声音。
**三项产品链路已接入；本机原生与 QUIC 回环验证通过；跨设备双向验收待第二台 Mac。**
原先的 macOS 服务启动阻塞已修复。本记录替代本日最初的预检结论。

## 基线与范围

- main 已从 `9d7e5987b83759db65eb815182eb83e9daf9b116` 安全快进至
  `d85ae3c614889979dfaf293ab6e217f1333b0e4c`，本轮实现尚未提交、推送。
- 原有四个 `junk/agent-python/cpp_capture/build/` 删除、`apps/Rdesk/.env.ios.local`
  和 `docs/plans/2026-09-07-macos-windows-lan-e2e-plan.md` 保留。
- 本机为 arm64 MacBook Pro，macOS 27.0。Mac 系统音频实现使用 ScreenCaptureKit。
- 参考 R-ShareMouse 的 `crates/rshare-platform/src/macos.rs`、
  `apps/rshare-daemon/src/audio_runtime.rs`、`crates/rshare-audio/src/media.rs`：
  采用统一按键语义、原生坐标转换、非阻塞音频回调、有限队列及旧音频丢弃原则。
  MRD 实现复用自身签名会话授权和 QUIC，不依赖另一个仓库的可执行文件。

## 已实现

### macOS 服务与身份

`apps/mrd-service/src/macos_runtime.rs` 接入现有 IPC、签名发现、持久授权和审计状态。
`security/macos_keychain.rs` 在登录钥匙串保存 AES-GCM 包装密钥，SQLite 中的秘密按用途密封；
密钥缺失但数据库仍存在时拒绝重建身份。数据目录为 0700，数据库及 IPC socket 为 0600。
单实例文件锁阻止重复服务启动，SIGTERM/退出时清理音频和按住的输入。
Mac 默认设备 ID 从持久公钥指纹派生，不依赖 Finder 未提供的 HOSTNAME 环境变量。
客户端通过 Tauri 2 适配器读取硬件信息，并使用服务的权威设备 ID；服务启动中会自动重试。

首次网络探测失败时保留 IPC 并由现有发现循环重试，避免 macOS 初次权限响应期间
出现 No route to host 就退出整个服务。

Unix IPC 现在拒绝覆盖活跃 socket 或普通文件，只回收失效 socket。
修复了原有 IPC 单元测试使用产品默认端点、导致运行中服务失联的问题；测试改用独立端点。

### 鼠标与键盘

`crates/mrd-input/src/macos.rs` 实现 CoreGraphics 原生输入：

- 指针、左右/中/扩展鼠标键、双击计数、按住按钮拖动。
- 垂直/水平滚轮；保留不足一个刻度的余量。
- Windows VK 协议到 Mac 硬件键码映射，支持字母、数字、标点、功能键、方向键、数字键盘和修饰键。
- 跟踪修饰键及重复按键；沿用服务的会话输入归属、授权、重放防护和断线释放。
- 将远端帧坐标映射到选中显示器/窗口的 macOS 逻辑点坐标，处理 Retina 比例及屏幕原点。
- 检查实际进程的辅助功能权限；未授权时返回明确错误。

补齐 Tauri 安全 IPC 白名单中的 `SendControlInput`，让现有前端按键/鼠标发送路径可达服务。
远端输入仍由服务验证已签名、未过期、具有对应权限的会话。

### 系统声音

`crates/mrd-audio` 提供 ScreenCaptureKit 系统声采集、CoreAudio 输出以及 PCM 协议和播放队列。
`apps/mrd-service/src/session_audio.rs` 将其接入当前会话的 QUIC DATAGRAM 和媒体复用层：

- 48 kHz、双声道、PCM16 LE；每包 5 ms、1000 字节，约 1.6 Mbps 应用层流量。
- 音频流绑定 grant ID 和 session ID；接收时校验格式、长度和流标识。
- 初始缓冲 20 ms，播放队列最多 60 ms；拒绝重复/过期包，丢包补静音，长中断后重新缓冲。
- 原生回调不等待网络；采集与接收队列有上限，发送时丢弃超过 50 ms 的排队音频。
- 排除服务自身播放声音，防止回授。采集停滞、权限或设备错误可查询，不伪装为正在播放。
- `audio.listen` 必须在被控端单独批准；仅查看屏幕不会启用声音。撤销、过期或断开会关闭音频。
- 前端按对端音频能力请求此权限，远程窗口显示实际声音状态、错误和静音按钮。

这是远程会话中的系统声播放；当前输出设备需支持 48 kHz 双声道 Float32。
缓冲大小不是端到端时延；双机时延、音画同步、蓝牙/单声道设备兼容性仍需实测。

## 验证结果

| 检查 | 结果 | 证据边界 |
| --- | --- | --- |
| 前端类型检查、生产构建 | PASS | 有现存大 chunk 提示 |
| 前端 Vitest | 55 文件、710 测试通过 | 含音频授权、状态、静音与错误交互 |
| mrd-service 库测试 | 608 通过、4 默认忽略 | 其中新增的 2 个原生音频测试另行显式运行 |
| mrd-input | 6 单元测试通过 | 映射、拖动、修饰键及原有输入逻辑 |
| mrd-audio | 4 单元测试通过 | 包边界、授权流绑定、乱序/重放、丢包及长中断 |
| mrd-ipc | 69 测试通过 | 14 库测试、47 合同、8 集成；含活跃端点保护 |
| 服务专项合同 | 60 通过 | 认证输入 16、IPC 5、持久身份 12、授权 15、签名发现 12 |
| 原生键鼠显式测试 | 8/8 事件被 macOS 接收 | 移动、按下/释放、拖动、按键按下/释放、双向滚轮；事件在 tap 中隔离，未输入日常应用 |
| 原生系统声采集/播放 | PASS | 4 秒采集 809 包，PCM 峰值 6498；CoreAudio 消费 380256 样本；10 丢弃、1 次欠载 |
| 原生音频授权/静音/撤销 | PASS | 12 包进入接收端，静音实际生效，撤销关闭输出及接收队列 |
| 原生声音经 QUIC mux | PASS | 最终复测发送 400、接收 398、峰值 6498；2 丢弃、2 次欠载；两端撤销后任务停止 |
| 签名服务实际启动 | PASS | IPC 可用、发现运行；命令行启动上下文键鼠权限 available，GUI 启动上下文见下方权限边界 |
| 持久身份重启 | PASS | 重启前后公共身份快照相同；0700/0600 权限验证通过 |
| 重复服务启动 | 正确拒绝 | 单实例锁返回已运行，现有服务保持可达 |
| 未授权 IPC 输入/声音 | 正确拒绝 | 无会话输入返回错误；声音返回 not_authorized |
| Tauri 原生客户端构建 | PASS | debug 客户端连接到本机真实服务 |
| 补丁空白检查 | PASS | `git diff --check` |

音频回环使用本机两个状态实例和真实 QUIC 连接，不能替代两台物理 Mac。
本轮没有声称物理耳听、端到端音画同步、中文 IME、多屏拖拽或双向 WAN 已通过。
通过 Finder/打包应用启动的 helper 与命令行测试的权限上下文不同：真实 GUI 服务
已返回辅助功能 PermissionMissing；本地网络设置中的 Rdesk 为开启状态。
已请求用户确认授予最终应用辅助功能及录屏/系统录音权限，尚未修改这些设置。
现存编译警告仍存在；此次没有进行全工作区、Windows 或 Linux 验收。

日志目录：`artifacts/e2e/2026-09-29-mac-input-audio/`（被 Git 忽略）：
`native-input.log`、`native-audio.log`、`native-quic-audio.log`、`audio-final.log`、
`service-tests-final.log`、`core-tests.log`、`ipc-final.log`、`contracts.log`、
`runtime-before-restart.json.log`、`restart-check.json.log`、`duplicate-service.log`、
`service-bundle.log`、`service-restarted.log`、`rdesk-app.log`。

## 两台 Mac 的操作及验收

1. 两端部署同一版本；在“隐私与安全性”中为最终的 Rdesk Service 应用授予辅助功能、屏幕录制/系统声音录制权限。
   权限属于实际签名和路径；重签名或换路径后应重新检查。
2. 打开 Rdesk，确认服务就绪；局域网发现使用 UDP 21116，建立会话后使用协商的 QUIC 端口。
3. 选中真实对端，发起 LAN 会话；被控端明确勾选指针、键盘和“系统声音”后同意。
4. 按下表分别执行 A→B 和 B→A，并记录双端状态及可观察效果。

| 验收项 | A→B | B→A |
| --- | --- | --- |
| 真实设备发现、身份匹配和同意/拒绝 | 待实测 | 待实测 |
| 鼠标左右/中键、双击、拖拽、双向滚轮 | 待实测 | 待实测 |
| 字母/标点/功能键、Command/Option/Control/Shift、中文输入法 | 待实测 | 待实测 |
| Retina、多显示器及窗口坐标、焦点切换 | 待实测 | 待实测 |
| 对端系统声音、静音、音画同步、时延 | 待实测 | 待实测 |
| 撤销、断网、关窗后停止播放并释放按键 | 待实测 | 待实测 |

当前真实 MRD 发现结果为 0 个对端。已将 Codex 连接列表中的 `lichengxudeMac-mini.local`
解析为 `192.168.2.58`；该主机 SSH 握手可达，但本机没有已信任的主机密钥和确认的登录配置，
严格主机验证未通过，因此没有登录或部署。双机验收仍需确认目标 Mac 及可用访问方式。
剪贴板、跨机文件传输和远程电源控制仍属于后续完整功能范围，本轮未实现。

## 复现测试

```bash
cargo test -p mrd-service --lib
cargo test -p mrd-input -p mrd-audio -p mrd-ipc
cargo test -p mrd-input --test macos_native -- --ignored --nocapture
cargo test -p mrd-audio --test macos_runtime -- --ignored --nocapture
cargo test -p mrd-service --lib session_audio -- --include-ignored --nocapture
```

最后三条在已登录的 Mac 桌面执行；原生音频测试会播放系统测试音。
发布构建继续使用 `cd apps/Rdesk && pnpm tauri:build --bundles app`。

## 本机构建产物

可运行的 debug 应用：`target/debug/bundle/macos/Rdesk.app`，内含最新 `MrdService.app`。
应用和内置服务均已进行 ad-hoc 签名并通过 `codesign --verify --deep --strict`。
此构建使用内嵌前端（`tauri://localhost`），不依赖 Vite 开发服务器。
本地 debug 打包使用资源覆盖后复制 debug helper 到 `Contents/Resources/MrdService.app`；
标准 release 打包仍使用仓库的 release helper 资源配置。

最终 GUI 核验：Rdesk 显示 `mrd-service online`、本机名 `1-6deMacBook-Pro.local`、
设备 ID `lan-c931fe019322b18e4a0e627e`；再次重启 GUI 服务后身份保持一致。
最终 IPC 返回 `control.keyboard_mouse = permission_missing`，给出辅助功能授权及重启提示。
广播成功时间戳持续更新，MRD 对端数为 0。记录见 `runtime-final.json.log` 和 `build-manifest.json.log`。
