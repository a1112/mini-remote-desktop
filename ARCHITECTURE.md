# Mini Remote Desktop - 项目架构

## 项目概述

Mini Remote Desktop 是一个基于 Rust 的高性能远程桌面系统，支持 WebRTC、QUIC、WebTransport 等多种传输协议。

```
mini-remote-desktop/
├── agent-rust/          # Rust Agent（被控端）
├── agent-python/        # Python Agent（被控端）
├── controller-rust/     # Rust Controller（控制端）
├── client-qt/           # Qt Controller（控制端）
├── web-client/          # Web Controller（控制端）
├── Rdesk/               # Tauri Desktop App（桌面客户端）
├── Rdesk-Server/        # FastAPI 后端服务器
├── signaling-rs/        # Rust 信令服务器
├── heartbeat-rs/        # UDP 心跳服务器
└── common-control-proto/ # 控制协议定义
```

## 核心组件

### 1. Agent（被控端）

#### agent-rust
负责捕获屏幕、编码视频、发送到控制端。

```
src/
├── main.rs              # 主入口，WebSocket 连接处理
├── capture_runtime.rs   # 屏幕捕获运行时（DXGI/WGC）
├── encoder_runtime.rs   # 编码器运行时（NVENC/QSV/AMF）
├── quic_tx.rs           # QUIC 传输发送
├── webtransport_tx.rs   # WebTransport 传输发送
├── input_injector.rs    # 键盘鼠标注入
├── clipboard.rs         # 剪贴板同步
├── file_ops/            # 文件操作
└── webdav_mount/        # WebDAV 挂载
```

**特性**：
- 支持多种捕获后端：DXGI、WGC、PowerShell
- 支持多种编码器：NVENC、QSV、AMF、openh264
- 支持多种传输：WebRTC、QUIC、WebTransport
- 动态帧率调整（基于网络条件）
- ROI（感兴趣区域）编码优化

#### agent-python
Python 实现的被控端，功能相对简化。

### 2. Controller（控制端）

#### controller-rust
Rust 实现的控制端，负责接收视频流、发送输入事件。

```
src/
├── main.rs              # 主入口
├── signaling/           # 信令客户端
│   ├── client.rs        # WebSocket 信令连接
│   └── protocol.rs      # 信令协议定义
├── render/              # 视频渲染
│   ├── d3d11.rs         # Direct3D 11 渲染
│   └── mod.rs
├── webrtc/              # WebRTC 实现
│   └── peer.rs          # WebRTC Peer Connection
├── input/               # 输入处理
├── stats/               # 统计信息
└── quic_rx.rs           # QUIC 传输接收
```

#### Rdesk（Tauri Desktop）
基于 Tauri 的桌面应用，使用 React + Vite 构建。

```
src-tauri/
├── src/
│   ├── main.rs          # Tauri 主入口
│   └── device_info.rs   # 设备信息获取（用于注册）
src/app/
├── components/          # React 组件
│   ├── DevicesPage.tsx
│   ├── DeviceDetailPage.tsx
│   ├── AuthModal.tsx
│   └── ...
└── services/            # 前端服务
    └── deviceService.ts # 设备注册服务
```

### 3. 服务器组件

#### Rdesk-Server（FastAPI 后端）
提供 REST API 和用户管理功能。

```
app/
├── api/v1/
│   ├── auth.py          # 认证 API
│   ├── devices.py       # 设备管理 API（注册/绑定）
│   ├── sessions.py      # 会话管理 API
│   └── users.py         # 用户管理 API
├── core/
│   ├── config.py        # 配置
│   └── security.py      # JWT 认证
├── models/
│   ├── user.py          # 用户模型
│   ├── device.py        # 设备模型（含绑定字段）
│   └── session_request.py
└── schemas/             # Pydantic schemas
```

**核心功能**：
- 用户认证（JWT）
- 设备注册（根据主板序列号生成 12 位纯数字设备 ID）
- 设备绑定（用户-设备关联）
- 会话管理

**设备 ID 生成规则**：
```python
# SHA256(主板序列号) → 大整数 → 取模 10^12 → 12 位纯数字
"BASEBOARD-12345" → "497770222245"
"WIN-ABC-123-DEF" → "052180600529"
```
- 格式：12 位纯数字（000000000001 - 999999999999）
- 算法：SHA256 → 整数转换 → 模 10^12
- 唯一性：基于 256 位哈希，碰撞概率约 1/10^12

#### signaling-rs（信令服务器）
WebSocket 信令服务器，负责设备注册、信令转发。

```
src/main.rs
├── 设备注册与管理
├── WebRTC Offer/Answer 转发
├── ICE 候选转发
├── 设备列表广播
└── 超时清理
```

**端口**：9527（WebSocket）、9528（UDP 发现）

#### heartbeat-rs（心跳服务器）
UDP 心跳服务器，轻量级设备在线状态维护。

```
src/
├── main.rs              # 服务器
└── client.rs            # 客户端库
```

**协议**：
- 端口：21114（UDP 心跳）、21115（UDP 发现）
- 间隔：30 秒
- 超时：60 秒

### 4. 协议定义

#### 信令协议
```json
// 设备注册
{"type": "device", "action": "register", "payload": {...}}

// WebRTC Offer
{"type": "webrtc", "action": "offer", "payload": {...}}

// ICE 候选
{"type": "webrtc", "action": "iceCandidate", "payload": {...}}
```

#### 心跳协议
```json
// 心跳消息
{
  "device_id": "F047A24581BD",
  "device_type": "agent",
  "device_name": "办公室电脑",
  "protocol_version": 2,
  "timestamp_ms": 1737269293000,
  "transports": ["webrtc", "quic"]
}
```

## 数据流

### 连接建立流程
```
1. Agent 启动 → 连接 signaling-rs（WebSocket）
2. Controller 启动 → 连接 signaling-rs（WebSocket）
3. 双方发送 register 消息注册
4. Agent 启动 heartbeat-rs 客户端，每 30s 发送 UDP 心跳
5. Controller 选择设备，发送 Offer
6. Agent 返回 Answer
7. ICE 候选交换
8. P2P 连接建立
9. 开始视频流传输
```

### 设备注册流程
```
1. Tauri 客户端启动
2. 获取硬件信息（主板序列号等）
3. 调用 Rdesk-Server /api/v1/devices/register
4. 服务器根据主板序列号生成设备 ID（SHA256[:12]）
5. 返回 device_id 和 access_token
6. 保存到本地存储
7. 后续启动自动验证并复用
```

## 传输协议

| 协议 | 端口 | 用途 | 优势 |
|------|------|------|------|
| WebSocket | 9527 | 信令控制 | 可靠、双向 |
| UDP | 21114 | 心跳保活 | 低开销 |
| WebRTC | 动态 | 视频流 | 低延迟、P2P |
| QUIC | 动态 | 视频流 | 高性能、多路复用 |
| WebTransport | 动态 | 视频流 | 低延迟、基于 QUIC |

## 配置文件

### signaling-rs
```toml
port = 9527
host = "0.0.0.0"
heartbeat_interval_secs = 30
connection_timeout_secs = 60
discovery_enable = true
discovery_port = 9528
```

### heartbeat-rs
```toml
udp_port = 21114
websocket_port = 9527
host = "0.0.0.0"
heartbeat_interval_secs = 30
connection_timeout_secs = 60
```

## 运行

### 启动顺序
```bash
# 1. PostgreSQL
# 2. Rdesk-Server（后端 API）
cd Rdesk-Server && python -m uvicorn app.main:app --port 9530 --reload

# 3. signaling-rs（信令服务器）
cd signaling-rs && cargo run

# 4. heartbeat-rs（心跳服务器）
cd heartbeat-rs && cargo run

# 5. agent-rust（被控端）
cd agent-rust && cargo run

# 6. Rdesk（控制端）
cd Rdesk && npm run tauri dev
```

## 设计参考

本项目参考了 [RustDesk](https://github.com/rustdesk/rustdesk) 的以下设计：

1. **设备 ID 生成**：SHA256 哈希硬件特征（主板序列号）
2. **UDP 心跳**：轻量级在线状态维护
3. **双服务器架构**：信令服务器 + 心跳服务器分离
4. **NAT 穿透**：WebRTC ICE + STUN/TURN 支持
5. **多协议支持**：同时支持 WebRTC、QUIC、WebTransport
