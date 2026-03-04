# FileMount 5/6/7 执行结果与 WebDAV 挂载调研

日期：2026-03-04
项目：`mini-remote-desktop/agent-rust`

## 1. 5/6/7 执行结果（落地状态）

对应 `FILEMOUNT_WEBDAV_SESSION_DESIGN.md` 的 5/6/7：

- 5（低耦合分层）: 已落目录骨架
  - `src/control_plane/*`
  - `src/webdav_mount/*`
  - `src/webdav_client/*`
  - `src/file_ops/*`
  - `src/security/*`
- 6（与现有代码对接点）: 已接入 `input_injector`
  - `ControlEvent::FileMount` -> `MountDispatcher::on_file_mount`
  - `FileControl/FileChunk` 组包完成后尝试解析 `MountEnvelope` -> `MountDispatcher::on_mount_envelope`
- 7（最小实现切片）: 已完成切片 #1 的主链路
  - `MOUNT_OPEN/CLOSE/HEARTBEAT/CAPS/LIST`
  - 会话状态机（`Init/Opening/Mounted/Degraded/Closing/Closed`）
  - 心跳超时降级逻辑

当前验证结果：
- `cargo test` (agent-rust) 通过
- `cargo check` (agent-rust) 通过

## 2. 关键接口变化

### 2.1 MountEnvelope

文件：`src/webdav_mount/envelope.rs`
- 新增字段：`flags: u32`（`serde(default)`，兼容旧 envelope）
- 保持 JSON 反序列化入口：`MountEnvelope::from_bytes`

### 2.2 Dispatcher

文件：`src/control_plane/dispatcher.rs`
- 新增 `on_mount_envelope(&MountEnvelope)`：
  - kind -> op 映射：`open/list/close/heartbeat/caps(_query)`
  - 仅 `open` 使用 `root_path`，其它 op 使用空 path

### 2.3 Input Injector

文件：`src/input_injector.rs`
- `FileControl` 完成时：
  - 若 payload 可解析为 `MountEnvelope`，走 mount dispatcher
  - 若不可解析，按普通文件传输完成处理（非致命回退）

## 3. Rust 开源 WebDAV 客户端（候选）

说明：以下数据来自 crates.io / GitHub（2026-03-04 采集）。

### 3.1 reqwest_dav（推荐优先验证）
- crate: `reqwest_dav` `0.3.3`
- repo: https://github.com/niuhuan/reqwest_dav
- docs: https://docs.rs/reqwest_dav/0.3.3
- 许可：MIT OR Apache-2.0
- crates updated_at: `2026-03-02`
- downloads: `615263`（recent: `165362`）
- 特点：tokio + reqwest，异步客户端，活跃度最高。

### 3.2 remotefs-webdav
- crate: `remotefs-webdav` `0.2.0`
- repo: https://github.com/remotefs-rs/remotefs-rs-webdav
- docs: https://docs.rs/remotefs-webdav/0.2.0
- 许可：MIT
- crates updated_at: `2024-09-30`
- downloads: `9890`（recent: `1040`）
- 特点：适合与 remotefs 抽象统一。

### 3.3 webdav-request
- crate: `webdav-request` `0.4.0`
- repo: https://github.com/cradiy/webdav-request
- docs: https://docs.rs/webdav-request/0.4.0
- 许可：MIT
- crates updated_at: `2025-12-03`
- downloads: `6293`（recent: `33`）
- 特点：轻量请求封装。

### 3.4 webdavc
- crate: `webdavc` `0.1.1`
- repo: https://github.com/dbian/rustydav_async
- docs: https://docs.rs/webdavc
- 许可：GPL-3.0
- crates updated_at: `2022-12-31`
- downloads: `2780`（recent: `10`）
- 特点：较老，GPL 许可可能影响商用闭源集成。

## 4. rclone 挂载到电脑方案

已克隆：`J:\ProjectTest\remote-desktop\_research\rclone`（HEAD: `78a7d9b`）

文档依据：
- `docs/content/commands/rclone_mount.md`
- `docs/content/webdav.md`
- `docs/content/commands/rclone_serve_webdav.md`

### 4.1 方案 A（推荐）：rclone 直挂本机盘符

前提：Windows 安装 WinFsp。

示例：

```powershell
# 先配置一个 webdav remote（交互式）
rclone config

# 挂载到盘符 X:
rclone mount mydav:/ X: --vfs-cache-mode writes --volname MRD_DAV

# 或挂载为网络盘模式
rclone mount mydav:/ X: --network-mode --volname \\cloud\mrd_dav
```

优点：
- 不依赖系统 WebClient 的 WebDAV 实现。
- 挂载行为可控，适合 agent 进程托管。

注意：
- Windows 下 `rclone mount` 为前台模式；建议由守护/服务管理拉起。
- `--network-mode` 只能挂盘符，不能挂目录路径。

### 4.2 方案 B：rclone serve webdav + 系统 WebDAV 映射

```powershell
# 本机暴露 WebDAV 服务
rclone serve webdav mydav:/ --addr :19080 --user u --pass p

# 然后用系统映射（资源管理器/网络位置）挂载 http://127.0.0.1:19080
```

注意：
- Windows 默认对 BasicAuth over HTTP 有限制。
- `BasicAuthLevel` 需要按文档调整（或使用 HTTPS）。
- 对 Office/WebClient 还可能需要额外注册表配置。

### 4.3 方案 C：agent-rust 直接 WebDAV 客户端 + 本地虚拟文件系统

- 短期：先用方案 A（rclone）快速得到“挂载可用”。
- 中期：agent-rust 内置 `WebDavClient`（`reqwest_dav`/自研）+ 会话管理。
- 长期：若要深度系统集成（资源管理器盘符一致体验），需引入 WinFsp/驱动层适配。

## 5. dav-server-rs 结论（与客户端边界）

已克隆：`J:\ProjectTest\remote-desktop\_research\dav-server-rs`（HEAD: `e3b5fd5`）

结论：
- `dav-server-rs` 是 **WebDAV 服务器库**，不是客户端库。
- 强项：可把本地/抽象存储暴露成 WebDAV（含 `DavFileSystem`、locksystem、actix/warp 适配）。
- 对你当前目标（agent 作为挂载客户端）可借鉴其协议处理与锁模型，但不能直接替代客户端实现。
