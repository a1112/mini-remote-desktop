# agent-rust 支持列表与扩展方向（2026-03-04）

## 当前支持列表

### 1) 传输与会话
- WebRTC 视频主链路（H264）
- QUIC 发送链路（实验/并行路径）
- WebTransport 发送链路（实验/并行路径）
- 信令注册能力上报（`webrtc/quic/webtransport`）
- 控制通道双路接收：`ctrl_rt`（实时）/`ctrl_rel`（可靠）

### 2) 控制协议（common-control-proto）
- 协议版本：`v2`
- 已支持事件：
  - 鼠标：移动、按键、滚轮
  - 键盘：按下/抬起
  - 手柄：轴/按键（协议已支持）
  - 剪贴板：`ClipboardSet` / `ClipboardGet`
  - 文件：`FileControl` / `FileChunk`
  - 音频控制：`AudioControl`
- 事件分类：
  - `Realtime`：MouseMove / MouseWheel / GamepadAxis
  - `Reliable`：其余事件

### 3) Agent 注入与处理
- Windows `SendInput`：鼠标 + 键盘注入已实现
- 手柄注入：当前为 stub（记录告警，未接入虚拟手柄驱动）
- 剪贴板：已接入事件处理，当前为进程内缓存（历史队列）
- 文件传输：已接入控制与分片重组（内存态），支持 begin/chunk/complete/cancel 基础流
- 音频控制：已接入会话参数管理（内存态）

### 4) 观测与验证
- 控制路径延迟统计：`[CTRL-LAT]` 每秒输出 `P50/P95/P99`
- 本地验证结果：
  - `common-control-proto`: `cargo test` 通过
  - `agent-rust`: `cargo check` / `cargo test` 通过

## 扩展方向（设计 1 / 2 / 3）

### 设计 1：剪贴板系统级落地（跨平台最小闭环）
目标：把当前“内存态剪贴板”升级为“系统剪贴板读写”。
- Windows: `OpenClipboard/GetClipboardData/SetClipboardData`
- Linux/macOS: 使用平台适配层（x11/wayland/pbcopy）
- 增加 `ClipboardGet` 的响应回传通道（当前仅本地处理）

已执行：
- 协议与 agent 事件入口已打通（`ClipboardSet/Get`）
- 当前状态可用于联调和压测，但不具备系统粘贴板互通

### 设计 2：文件传输可靠化（可恢复、可校验）
目标：把当前“内存重组”升级为“可断点恢复 + 落盘 + 校验”。
- 增加 transfer 元信息（文件名、大小、mtime、权限）
- 分片 ACK/NACK 与重传窗口
- 断线重连后按 `transfer_id + chunk_idx` 续传
- 完整 SHA-256 校验（当前为 16 字节摘要占位）

已执行：
- `FileControl/FileChunk` 编解码完成
- agent 侧 begin/chunk/complete/cancel 流程可跑通

### 设计 3：手柄与音频控制生产化
目标：完成“可用输入外设 + 音频参数联动”的生产链路。
- 手柄：接入 ViGEm（Windows）实现虚拟 XInput 注入
- 音频：将 `AudioControl` 与采集/编码配置联动（codec/rate/ch/frame）
- 增加端到端兼容矩阵（Xbox/DS 系列映射）

已执行：
- 协议已支持 `Gamepad*` 与 `AudioControl`
- agent 侧 `AudioControl` 已有状态管理入口
- 手柄仍是 stub，需下一阶段驱动级实现

## 建议优先级
1. 先做设计 2（文件可靠化）：收益最大，且不依赖驱动。
2. 再做设计 1（系统剪贴板）：快速形成用户可感知功能闭环。
3. 最后做设计 3（ViGEm + 音频联动）：实现复杂度最高，适合在前两项稳定后推进。
