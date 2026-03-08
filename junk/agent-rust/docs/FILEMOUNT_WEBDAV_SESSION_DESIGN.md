# FileMount -> WebDAV 挂载会话接口设计（低耦合 / 分文件夹）

日期：2026-03-04
目标项目：`mini-remote-desktop/agent-rust`

## 1. 目标与约束

### 1.1 目标
- 通过 `FileMount` 控制事件建立/维护/关闭 WebDAV 挂载会话。
- 保持 `agent-rust` 低耦合：协议层、会话层、WebDAV 客户端层、文件操作层分离。
- 支持“一会话一根目录（folder-scoped mount）”，避免跨目录耦合和权限蔓延。

### 1.2 非目标
- 本阶段不实现完整虚拟盘驱动（仅会话与文件 API 层）。
- 不在控制层直接绑定具体 WebDAV SDK（通过 trait 抽象）。

---

## 2. 协议映射

## 2.1 基础事件（已存在）
使用 `common-control-proto::ControlEvent::FileMount`：
- `op: u8`
- `mount_id: u64`
- `flags: u32`
- `path: String`

语义：
- `mount_id`：会话主键，端到端唯一。
- `path`：挂载根目录（相对 WebDAV 根），例如 `/team/docs/`。
- `flags`：读写策略与能力开关（见下）。

## 2.2 FileMount 操作码
- `0x01 MOUNT_OPEN`
- `0x02 MOUNT_LIST`
- `0x03 MOUNT_CLOSE`
- `0x04 MOUNT_HEARTBEAT`
- `0x05 MOUNT_CAPS_QUERY`

## 2.3 flags 位定义
- bit0 `READ_ONLY`：只读会话
- bit1 `AUTO_CREATE_ROOT`：根目录不存在时自动创建
- bit2 `ALLOW_DELETE`：允许删除
- bit3 `ALLOW_MOVE`：允许移动/重命名
- bit4 `ALLOW_OVERWRITE`：允许覆盖写
- bit5 `STRICT_ETAG`：强 ETag 前置条件

## 2.4 扩展请求字段（建议）
`FileMount` 只承载会话控制；详细请求参数通过 `FileControl + FileChunk` 传输 `MountEnvelope`：

`MountEnvelope`（JSON）
- `version: u16`
- `mount_id: u64`
- `request_id: u64`
- `kind: "open" | "op" | "close" | "heartbeat"`
- `auth`（仅 open 时）
  - `url: string`
  - `username: string`
  - `password_ref: string`（密文句柄/临时令牌）
- `root_path: string`
- `op`（kind=op 时）
  - `name: "stat"|"list"|"read"|"write"|"mkdir"|"delete"|"move"|"copy"`
  - `path: string`
  - `dst_path?: string`
  - `offset?: u64`
  - `length?: u64`
  - `etag?: string`

说明：控制平面仍由 `FileMount` 驱动；业务数据平面统一 `MountEnvelope`，避免继续膨胀 `ControlEvent`。

---

## 3. 会话状态机

状态：
- `INIT`
- `OPENING`
- `MOUNTED`
- `DEGRADED`
- `CLOSING`
- `CLOSED`
- `ERROR`

转移：
- `INIT --MOUNT_OPEN--> OPENING`
- `OPENING --auth+probe ok--> MOUNTED`
- `OPENING --auth/probe fail--> ERROR`
- `MOUNTED --heartbeat timeout--> DEGRADED`
- `DEGRADED --heartbeat resume--> MOUNTED`
- `MOUNTED/DEGRADED --MOUNT_CLOSE--> CLOSING --> CLOSED`
- 任意状态出现不可恢复错误：`-> ERROR`

超时建议：
- 心跳间隔：`5s`
- 心跳超时：`15s`
- 会话回收：`60s`

---

## 4. 错误码设计

范围分层：
- `1xxx` 协议层
- `2xxx` 会话层
- `3xxx` WebDAV 访问层
- `4xxx` 文件操作层

建议错误码：
- `1001 INVALID_OP`
- `1002 INVALID_FIELD`
- `1003 VERSION_MISMATCH`
- `2001 MOUNT_NOT_FOUND`
- `2002 MOUNT_ALREADY_EXISTS`
- `2003 MOUNT_STATE_CONFLICT`
- `2004 HEARTBEAT_TIMEOUT`
- `3001 AUTH_FAILED`
- `3002 DAV_UNREACHABLE`
- `3003 DAV_TIMEOUT`
- `3004 DAV_HTTP_4XX`
- `3005 DAV_HTTP_5XX`
- `4001 PATH_FORBIDDEN`
- `4002 NOT_FOUND`
- `4003 ALREADY_EXISTS`
- `4004 PRECONDITION_FAILED`
- `4005 QUOTA_EXCEEDED`
- `4006 LOCKED`

返回结构（建议）：
- `code: u32`
- `message: string`
- `http_status?: u16`
- `retryable: bool`

---

## 5. agent-rust 低耦合落地（分文件夹）

目录建议：

```text
agent-rust/src/
  control_plane/
    mod.rs
    dispatcher.rs          # ControlEvent -> 子域路由
    mount_protocol.rs      # op/flag/错误码常量

  webdav_mount/
    mod.rs
    session.rs             # MountSession 状态机
    manager.rs             # mount_id -> session map / lifecycle
    envelope.rs            # MountEnvelope 序列化/反序列化

  webdav_client/
    mod.rs
    trait.rs               # WebDavClient trait（低耦合核心）
    reqwest_impl.rs        # 具体 HTTP/WebDAV 实现
    model.rs               # Stat/ListEntry/ReadResult 等 DTO

  file_ops/
    mod.rs
    service.rs             # 业务操作编排：stat/list/read/write...
    policy.rs              # 路径策略、权限策略、并发策略

  security/
    mod.rs
    secret_store.rs        # password_ref -> 明文凭据（短生命周期）
```

依赖方向（必须单向）：
- `control_plane -> webdav_mount -> file_ops -> webdav_client`
- `security` 只被 `webdav_mount/file_ops` 调用
- `webdav_client` 不反向依赖 `control_plane`

这样可以做到：
- 更换 WebDAV SDK 仅改 `webdav_client/*`
- 协议升级主要改 `control_plane/*` 和 `webdav_mount/envelope.rs`
- 文件策略变更只改 `file_ops/policy.rs`

---

## 6. 与现有代码对接点

### 6.1 `input_injector.rs`
- 仅做路由，不做业务：
  - `ControlEvent::FileMount` -> `control_plane::dispatcher::on_file_mount(...)`
  - `ControlEvent::FileControl/FileChunk` -> `control_plane::dispatcher::on_mount_envelope(...)`

### 6.2 `file_transfer.rs`
- 保持原有传输职责，不直接耦合 WebDAV。
- 仅复用分片传输通道承载 `MountEnvelope`（可选）。

### 6.3 `common-control-proto`
- 保留 `FileMount` 作为控制入口。
- 避免频繁新增 `ControlEvent`，优先通过 `MountEnvelope.kind/op` 扩展。

---

## 7. 最小实现切片（建议顺序）

1. `MOUNT_OPEN/MOUNT_CLOSE/MOUNT_HEARTBEAT` + 状态机
2. `stat/list/read` 只读链路
3. `mkdir/write/delete/move/copy` 写链路 + `flags` 校验
4. ETag/锁/重试策略
5. 观测：`mount_active`, `mount_err_rate`, `dav_rtt_p95`

---

## 8. 接口示例

`FileMount(op=0x01,mount_id=10,flags=0b000001,path="/team/docs/")`

随后 `MountEnvelope(kind="open")`:

```json
{
  "version": 1,
  "mount_id": 10,
  "request_id": 1,
  "kind": "open",
  "auth": {
    "url": "https://dav.example.com/remote.php/dav/files/dev/",
    "username": "dev",
    "password_ref": "secret://session/abc123"
  },
  "root_path": "/team/docs/"
}
```

---

## 9. 兼容性策略

- 未识别 `op`：返回 `1001 INVALID_OP`，不影响现有会话。
- 未识别 `flags` 位：忽略并在响应能力中回显 `unsupported_flags`。
- `version` 不兼容：返回 `1003 VERSION_MISMATCH`，附 `supported_versions`。
