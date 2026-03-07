# Mini Remote Desktop

## Rebuild Notice

This repository is being rebuilt into a clean product-oriented workspace.

The long-term mainline is moving toward:

- `apps/Rdesk`
- `apps/Rdesk-Server`
- `apps/realtime-server`
- `crates/*`
- `labs/GPUTest`
- `junk/*`

Until the rebuild is complete, recovered trees and older projects should be treated as reference material, not as architecture-defining sources of truth.

极简高性能远程桌面方案 - 完全私有化部署

## 特点

- ✅ **极简架构** - 核心组件，可选数据库
- ✅ **高性能** - 原生 Rust 实现，硬件加速
- ✅ **P2P 连接** - 数据不经过服务器
- ✅ **多协议支持** - WebRTC、QUIC、WebTransport
- ✅ **完全私有** - 所有代码可控
- ✅ **跨平台** - Web、桌面、Qt 控制端

## 架构

```
┌─────────────────────────────────────────────────────────────────┐
│                         Mini Remote Desktop                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│   控制端 (Web)                                                  │
│   ┌────────────────────────────────────────────────────────────┐ │
│   │  WebRTC (视频/音频) + DataChannel (鼠标/键盘)             │ │
│   └────────────────────────────────────────────────────────────┘ │
│                           ↕ P2P                                  │
│   ┌────────────────────────────────────────────────────────────┐ │
│   │  信令服务器 (WebSocket) - 仅用于交换连接信息              │ │
│   │  ws://localhost:9527                                       │ │
│   └────────────────────────────────────────────────────────────┘ │
│                           ↕                                      │
│   被控端 (Electron)                                             │
│   ┌────────────────────────────────────────────────────────────┐ │
│   │  屏幕捕获 + 鼠标键盘模拟                                   │ │
│   └────────────────────────────────────────────────────────────┘ │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## 快速开始

### 1. 安装依赖

```bash
# 安装服务端依赖
cd server
npm install

# 安装被控端依赖
cd ../agent
npm install
```

### 2. 启动服务

**Windows:**
```bash
start-all.bat
```

**手动启动:**
```bash
# 终端 1: 启动信令服务器
cd server
node index.js

# 终端 2: 启动被控端
cd agent
npm start
```

### 3. 开始使用

1. 用浏览器打开 `web/index.html`
2. 看到在线设备后，点击连接
3. 开始远程控制

## 文件结构

```
mini-remote-desktop/
├── server/
│   ├── index.js      # 信令服务器 (~200 行)
│   └── package.json
├── web/
│   ├── index.html    # 控制端界面
│   └── app.js        # WebRTC 逻辑 (~400 行)
├── agent/
│   ├── main.js       # Electron 主进程
│   ├── index.html    # Agent 界面
│   ├── renderer.js   # 渲染进程
│   └── package.json
└── start-all.bat     # 一键启动脚本
```

## 技术栈

| 组件 | 技术 |
|------|------|
| 信令服务 | Node.js + WebSocket (ws) |
| 控制端 | 原生 JavaScript + WebRTC |
| 被控端 | Electron + robotjs |

## 性能优化

- **零依赖信令** - 原生 WebSocket，无 Socket.IO 开销
- **硬件加速** - 使用 GPU 编解码
- **P2P 传输** - 数据直连，不经过服务器
- **自适应码率** - 根据网络自动调整
- **UDP 优先** - WebRTC 默认使用 UDP

## 安全建议

- 生产环境使用 WSS (WebSocket Secure)
- 添加设备认证机制
- 使用防火墙限制端口访问
- 定期更新依赖版本

## 配置

修改 `web/app.js` 和 `agent/main.js` 中的 `WS_URL`：

```javascript
const CONFIG = {
  WS_URL: 'ws://your-server:9527'
};
```

Agent 采集参数改为配置文件 `agent/config.json`：

```json
{
  "wsUrl": "ws://localhost:9527",
  "capture": {
    "fps": 30,
    "minWidth": 1280,
    "maxWidth": 1920,
    "minHeight": 720,
    "maxHeight": 1080
  }
}
```

修改后重启 Agent 生效。

`agent-rust/config.json` 采集策略支持后端选择与回退：

```json
{
  "ws_url": "ws://127.0.0.1:9527",
  "device_name": "Rust Agent",
  "capture": {
    "fps": 8,
    "jpeg_quality": 70,
    "backend": "auto",
    "allow_fallback": true,
    "encoder": "auto",
    "allow_encoder_fallback": true
  }
}
```

- `backend`: `auto | dxgi | powershell | dummy`
- `allow_fallback`: `true` 时按策略回退；`false` 时只尝试指定后端
- `encoder`: `auto | nvenc | qsv | amf | openh264`
- `allow_encoder_fallback`: `true` 时编码后端按优先级回退

## 故障排除

**无法连接设备?**
- 检查信令服务器是否运行
- 检查防火墙设置
- 尝试使用公共 STUN 服务器

**画面卡顿?**
- 降低帧率设置
- 检查网络带宽
- 使用有线连接

**鼠标键盘无响应?**
- 检查 DataChannel 状态
- 确认被控端有权限

## 许可证

MIT

## Rust Agent 原型（实验）

新增目录：`agent-rust/`，用于验证 Rust 迁移路径（WS 信令 + 屏幕采集 + 画面推送）。

启动步骤：

```bash
# 终端 1
cd server
node index.js

# 终端 2
cd ../agent-rust
cargo run

# 终端 3（预览）
cd ../web
node server.js
# 打开 http://localhost:8080/rust-viewer.html
```

说明：
- 这是迁移原型，不替代现有 WebRTC Agent。
- 当前原型通过 `stream/frame` 中继 JPEG 帧，便于快速验证 Rust 端能力。

## signaling-rs（Rust 信令服务）

目录：`signaling-rs/`

启动：

```bash
cd signaling-rs
cargo run
```

默认监听：`ws://0.0.0.0:9527`

切换步骤：
1. 停止现有 Node 信令服务（占用 9527 的进程）。
2. 启动 `signaling-rs`。
3. 保持 `web` 与 `agent/agent-rust` 的 `ws://localhost:9527` 不变即可连接。

## heartbeat-rs（UDP 心跳服务）

目录：`heartbeat-rs/`

参考 RustDesk hbbs 设计，使用 UDP 实现轻量级心跳保活机制。

启动：

```bash
cd heartbeat-rs
cargo run
```

默认监听：`UDP 0.0.0.0:21114`

**特性**：
- UDP 心跳，低开销（30秒间隔）
- 维护设备在线状态和 IP 信息
- 60秒超时自动断开
- 支持 UDP 发现（端口 21115）

**协议**：

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

## Rdesk-Server（后端 API）

目录：`Rdesk-Server/`

FastAPI 后端，提供用户管理、设备注册等功能。

启动：

```bash
cd Rdesk-Server
python -m uvicorn app.main:app --host 0.0.0.0 --port 9530 --reload
```

**核心 API**：
- `POST /api/v1/devices/register` - 设备注册（匿名）
- `GET /api/v1/devices/check/{serial}` - 检查设备状态
- `POST /api/v1/auth/login` - 用户登录
- `GET /api/v1/users/me` - 用户信息

**设备 ID 生成**：
根据主板序列号生成 SHA256 哈希，取前12位作为设备 ID。

## 完整启动顺序

```bash
# 1. PostgreSQL（可选，用于用户管理）
# 2. Rdesk-Server（端口 9530）
cd Rdesk-Server && python -m uvicorn app.main:app --port 9530 --reload

# 3. signaling-rs（端口 9527 WebSocket, 9528 UDP 发现）
cd signaling-rs && cargo run

# 4. heartbeat-rs（端口 21114 UDP 心跳, 21115 UDP 发现）
cd heartbeat-rs && cargo run

# 5. agent-rust（被控端）
cd agent-rust && cargo run

# 6. Rdesk（Tauri 桌面控制端）
cd Rdesk && npm run tauri dev
```

## 更多文档

详细架构说明请参阅 [ARCHITECTURE.md](ARCHITECTURE.md)
