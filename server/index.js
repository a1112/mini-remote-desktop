/**
 * Mini Remote Desktop - 高性能信令服务端
 *
 * 架构：
 * - 纯 WebSocket (无 Socket.IO 开销)
 * - 内存存储 (无数据库)
 * - O(1) 设备查找
 */

import { WebSocketServer } from 'ws';
import { v4 as uuidv4 } from 'uuid';

// 配置
const PORT = 9527;
const MAX_DEVICES = 10000;

// 内存存储
const devices = new Map();      // deviceId → { ws, type, lastHeartbeat }
const connections = new Map();  // ws → deviceId
const pendingOffers = new Map(); // sessionId → { offer, controllerWs }

// 日志
const log = {
  info: (msg) => console.log(`[INFO] ${new Date().toLocaleTimeString()} ${msg}`),
  error: (msg) => console.error(`[ERROR] ${new Date().toLocaleTimeString()} ${msg}`),
  success: (msg) => console.log(`\x1b[32m[SUCCESS]\x1b[0m ${msg}`)
};

// 创建 WebSocket 服务器
const wss = new WebSocketServer({
  port: PORT,
  perMessageDeflate: false // 禁用压缩，提升性能
});

// 心跳检测 (每30秒)
setInterval(() => {
  const now = Date.now();
  for (const [deviceId, device] of devices) {
    if (now - device.lastHeartbeat > 60000) {
      log.info(`设备超时: ${deviceId}`);
      device.ws?.close();
      devices.delete(deviceId);
    }
  }
}, 30000);

// 广播给指定类型的设备
function broadcast(type, message, excludeWs = null) {
  const data = JSON.stringify(message);
  for (const [deviceId, device] of devices) {
    if (device.type === type && device.ws !== excludeWs && device.ws?.readyState === 1) {
      device.ws.send(data);
    }
  }
}

// 处理消息
function handleMessage(ws, deviceId, data) {
  const { type, action, payload } = data;

  switch (action) {
    // === 设备注册 ===
    case 'register': {
      const deviceType = payload.type; // 'controller' | 'agent'
      const deviceName = payload.name || `${deviceType}-${deviceId.slice(0, 8)}`;

      devices.set(deviceId, {
        ws,
        type: deviceType,
        name: deviceName,
        lastHeartbeat: Date.now()
      });
      connections.set(ws, deviceId);

      log.success(`${deviceType} 注册: ${deviceName} (${deviceId})`);

      // 发送设备列表
      const deviceList = Array.from(devices.entries())
        .filter(([_, d]) => d.type === 'agent' || d.type === 'agent-rust')
        .map(([id, d]) => ({ id, name: d.name, online: true }));

      ws.send(JSON.stringify({ type, action: 'registered', payload: { deviceId, deviceList } }));

      // 通知其他控制器更新列表
      broadcast('controller', { type: 'device', action: 'deviceList', payload: { deviceList } }, ws);
      break;
    }

    // === 心跳 ===
    case 'ping': {
      const device = devices.get(deviceId);
      if (device) {
        device.lastHeartbeat = Date.now();
        ws.send(JSON.stringify({ type, action: 'pong' }));
      }
      break;
    }

    // === WebRTC 信令 ===
    case 'offer': {
      // 控制端发起连接
      const targetId = payload.targetDeviceId;
      const targetDevice = devices.get(targetId);

      if (!targetDevice || targetDevice.type !== 'agent') {
        ws.send(JSON.stringify({ type: 'webrtc', action: 'error', payload: { message: '设备不存在' } }));
        return;
      }

      const sessionId = uuidv4();
      pendingOffers.set(sessionId, { controllerWs: ws, controllerId: deviceId });

      // 转发 offer 给被控端
      targetDevice.ws.send(JSON.stringify({
        type: 'webrtc',
        action: 'offer',
        payload: { ...payload, sessionId, controllerId: deviceId }
      }));

      log.info(`连接请求: ${deviceId} → ${targetId}`);
      break;
    }

    case 'answer': {
      // 被控端响应
      const { controllerId } = payload;
      const controllerDevice = devices.get(controllerId);

      if (controllerDevice && controllerDevice.type === 'controller' && controllerDevice.ws?.readyState === 1) {
        controllerDevice.ws.send(JSON.stringify({ type: 'webrtc', action: 'answer', payload }));
        log.info(`Answer 转发: ${controllerId}`);
      } else {
        log.error(`无法找到控制器: ${controllerId}`);
      }
      break;
    }

    case 'iceCandidate': {
      // ICE 候选转发
      const { controllerId, targetDeviceId, candidate } = payload;
      const targetDevice = devices.get(targetDeviceId);

      // 确保 candidate 有效且有必要的字段
      if (targetDevice?.ws?.readyState === 1 && candidate) {
        if (candidate.candidate || candidate.sdpMid || candidate.sdpMLineIndex) {
          targetDevice.ws.send(JSON.stringify({
            type: 'webrtc',
            action: 'iceCandidate',
            payload: { candidate, controllerId }
          }));
        }
      }
      break;
    }

    // === 设备控制 ===
    case 'frame': {
      const sender = devices.get(deviceId);
      if (!sender || (sender.type !== 'agent' && sender.type !== 'agent-rust')) return;

      // Rust 原型流：通过信令服务器中继 JPEG 帧到控制端
      broadcast('controller', {
        type: 'stream',
        action: 'frame',
        payload: {
          deviceId,
          deviceName: sender.name,
          ...payload
        }
      });
      break;
    }

    case 'getDeviceList': {
      const deviceList = Array.from(devices.entries())
        .filter(([_, d]) => d.type === 'agent' || d.type === 'agent-rust')
        .map(([id, d]) => ({ id, name: d.name, online: true }));

      ws.send(JSON.stringify({ type, action: 'deviceList', payload: { deviceList } }));
      break;
    }

    default:
      ws.send(JSON.stringify({ type: 'error', action: 'error', payload: { message: '未知操作' } }));
  }
}

// WebSocket 连接处理
wss.on('connection', (ws) => {
  const deviceId = uuidv4();
  log.info(`新连接: ${deviceId}`);

  ws.on('message', (data) => {
    try {
      const message = JSON.parse(data);
      handleMessage(ws, deviceId, message);
    } catch (err) {
      log.error(`消息解析错误: ${err.message}`);
    }
  });

  ws.on('close', () => {
    const id = connections.get(ws);
    if (id) {
      const device = devices.get(id);
      if (device) {
        log.info(`设备离线: ${device.name} (${id})`);
        devices.delete(id);
        // 广播设备离线
        broadcast('controller', {
          type: 'device',
          action: 'offline',
          payload: { deviceId: id }
        });
      }
      connections.delete(ws);
    }
  });

  ws.on('error', (err) => {
    log.error(`WebSocket 错误: ${err.message}`);
  });

  // 发送设备 ID
  ws.send(JSON.stringify({
    type: 'system',
    action: 'connected',
    payload: { deviceId }
  }));
});

// 启动服务器
wss.on('listening', () => {
  log.success(`信令服务器启动: ws://localhost:${PORT}`);
  log.info(`等待设备连接...`);
});

// 优雅关闭
process.on('SIGINT', () => {
  log.info('正在关闭服务器...');
  for (const ws of wss.clients) {
    ws.close();
  }
  wss.close(() => {
    log.success('服务器已关闭');
    process.exit(0);
  });
});
