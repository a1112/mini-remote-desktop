/**
 * Mini Remote Desktop - Web 控制端
 *
 * 高性能优化：
 * - 原生 WebSocket（无库开销）
 * - 硬件加速编解码
 * - 自适应码率
 * - 零延迟 DataChannel
 */
import { buildWsUrl } from './ws-url.js';

// ==================== 配置 ====================
const CONFIG = {
  WS_URL: buildWsUrl(window.location),
  // 公共 STUN 服务器（免费）
  ICE_SERVERS: [
    { urls: 'stun:stun.l.google.com:19302' },
    { urls: 'stun:stun1.l.google.com:19302' },
  ]
};

// ==================== 状态 ====================
const state = {
  ws: null,
  deviceId: null,
  devices: new Map(),
  selectedDevice: null,
  pc: null,
  dataChannel: null,
  remoteStream: null,
  statsInterval: null
};

// ==================== DOM 元素 ====================
const elements = {
  statusDot: document.getElementById('statusDot'),
  statusText: document.getElementById('statusText'),
  deviceList: document.getElementById('deviceList'),
  remoteVideo: document.getElementById('remoteVideo'),
  placeholder: document.getElementById('placeholder'),
  stats: document.getElementById('stats'),
  latency: document.getElementById('latency'),
  bitrate: document.getElementById('bitrate'),
  fps: document.getElementById('fps'),
  packetLoss: document.getElementById('packetLoss'),
  btnConnect: document.getElementById('btnConnect'),
  btnDisconnect: document.getElementById('btnDisconnect'),
  btnFullscreen: document.getElementById('btnFullscreen')
};

// ==================== WebSocket ====================
function connectWebSocket() {
  state.ws = new WebSocket(CONFIG.WS_URL);
  updateStatus('connecting');

  state.ws.onopen = () => {
    console.log('[WS] 连接成功');
  };

  state.ws.onmessage = async (event) => {
    try {
      const message = JSON.parse(event.data);
      handleMessage(message);
    } catch (err) {
      console.error('[WS] 消息解析错误:', err);
    }
  };

  state.ws.onclose = () => {
    console.log('[WS] 连接断开');
    updateStatus('offline');
    setTimeout(connectWebSocket, 3000);
  };

  state.ws.onerror = (err) => {
    console.error('[WS] 错误:', err);
  };
}

function handleMessage(message) {
  const { type, action, payload } = message;

  switch (type) {
    case 'system':
      if (action === 'connected') {
        state.deviceId = payload.deviceId;
        // 注册为控制器
        send({ type: 'device', action: 'register', payload: { type: 'controller', name: 'Web 控制端' } });
      }
      break;

    case 'device':
      if (action === 'registered') {
        updateStatus('online');
        updateDeviceList(payload.deviceList);
      } else if (action === 'deviceList') {
        updateDeviceList(payload.deviceList);
      } else if (action === 'offline') {
        removeDevice(payload.deviceId);
      }
      break;

    case 'webrtc':
      handleWebRTCMessage(action, payload);
      break;
  }
}

function send(message) {
  if (state.ws?.readyState === 1) {
    state.ws.send(JSON.stringify(message));
  }
}

// ==================== 设备列表 ====================
function updateDeviceList(devices) {
  state.devices.clear();
  devices.forEach(d => state.devices.set(d.id, d));

  if (devices.length === 0) {
    elements.deviceList.innerHTML = '<div style="padding:20px;text-align:center;color:#666">暂无在线设备</div>';
    return;
  }

  elements.deviceList.innerHTML = devices.map(d => `
    <div class="device-item ${state.selectedDevice === d.id ? 'connected' : ''}" data-id="${d.id}">
      <div class="device-icon">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
          <path d="M20 18c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2H4c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2H0v2h24v-2h-4zM4 6h16v10H4V6z"/>
        </svg>
      </div>
      <div class="device-info">
        <div class="device-name">${d.name}</div>
        <div class="device-status">在线</div>
      </div>
      <span class="badge online">ONLINE</span>
    </div>
  `).join('');

  // 绑定点击事件
  elements.deviceList.querySelectorAll('.device-item').forEach(el => {
    el.addEventListener('click', () => selectDevice(el.dataset.id));
  });
}

function removeDevice(deviceId) {
  state.devices.delete(deviceId);
  if (state.selectedDevice === deviceId) {
    disconnect();
    state.selectedDevice = null;
  }
  updateDeviceList(Array.from(state.devices.values()));
}

function selectDevice(deviceId) {
  state.selectedDevice = deviceId;
  updateDeviceList(Array.from(state.devices.values()));
  elements.btnConnect.disabled = false;
}

// ==================== WebRTC ====================
async function connect() {
  if (!state.selectedDevice) return;

  const device = state.devices.get(state.selectedDevice);
  if (!device) return;

  console.log('[WebRTC] 连接设备:', device.name);

  // 创建 RTCPeerConnection
  state.pc = new RTCPeerConnection({
    iceServers: CONFIG.ICE_SERVERS,
    // 优化配置
    rtcConfiguration: {
      iceTransportPolicy: 'all',
      bundlePolicy: 'max-bundle',
      rtcpMuxPolicy: 'require'
    }
  });

  // 监听 ICE 候选
  state.pc.onicecandidate = (event) => {
    if (event.candidate) {
      send({
        type: 'webrtc',
        action: 'iceCandidate',
        payload: {
          targetDeviceId: state.selectedDevice,
          candidate: event.candidate
        }
      });
    }
  };

  // 监听连接状态
  state.pc.onconnectionstatechange = () => {
    console.log('[WebRTC] 连接状态:', state.pc.connectionState);
    if (state.pc.connectionState === 'connected') {
      onConnected();
    } else if (state.pc.connectionState === 'disconnected') {
      onDisconnected();
    }
  };

  // 监听 ICE 连接状态
  state.pc.oniceconnectionstatechange = () => {
    console.log('[WebRTC] ICE 状态:', state.pc.iceConnectionState);
  };

  // 监听远程流
  state.pc.ontrack = (event) => {
    console.log('[WebRTC] 收到远程流');
    state.remoteStream = event.streams[0];
    elements.remoteVideo.srcObject = state.remoteStream;
    elements.remoteVideo.style.display = 'block';
    elements.placeholder.style.display = 'none';
  };

  // 创建数据通道（用于鼠标键盘）
  state.dataChannel = state.pc.createDataChannel('control', {
    ordered: false, // 无序传输，降低延迟
    maxRetransmits: 0 // 不重传，实时性优先
  });

  state.dataChannel.onopen = () => {
    console.log('[DataChannel] 已打开');
  };

  state.dataChannel.onmessage = (event) => {
    // 处理来自被控端的消息
    try {
      const data = JSON.parse(event.data);
      console.log('[DataChannel] 收到:', data.type);
    } catch (e) {}
  };

  // 创建 Offer
  const offer = await state.pc.createOffer({
    offerToReceiveAudio: false,
    offerToReceiveVideo: true
  });

  await state.pc.setLocalDescription(offer);

  // 发送 Offer
  send({
    type: 'webrtc',
    action: 'offer',
    payload: {
      targetDeviceId: state.selectedDevice,
      offer: offer
    }
  });

  elements.btnConnect.disabled = true;
  elements.btnDisconnect.disabled = false;
  elements.btnFullscreen.disabled = false;
}

function handleWebRTCMessage(action, payload) {
  console.log('[WebRTC] 收到消息:', action, payload);

  switch (action) {
    case 'answer':
      console.log('[WebRTC] 收到 Answer');
      state.pc?.setRemoteDescription(new RTCSessionDescription(payload.answer))
        .then(() => console.log('[WebRTC] RemoteDescription 设置成功'))
        .catch(e => console.error('[WebRTC] RemoteDescription 错误:', e));
      break;

    case 'iceCandidate':
      if (payload.candidate && payload.candidate.candidate) {
        try {
          state.pc?.addIceCandidate(new RTCIceCandidate(payload.candidate));
        } catch (e) {
          console.error('[WebRTC] ICE candidate 错误:', e, payload.candidate);
        }
      }
      break;

    case 'offer':
      // 被控端不会收到 offer，但保留此逻辑
      break;

    case 'error':
      console.error('[WebRTC] 服务器错误:', payload.message);
      break;
  }
}

function disconnect() {
  if (state.pc) {
    state.pc.close();
    state.pc = null;
  }

  if (state.dataChannel) {
    state.dataChannel.close();
    state.dataChannel = null;
  }

  elements.remoteVideo.srcObject = null;
  elements.remoteVideo.style.display = 'none';
  elements.placeholder.style.display = 'block';

  elements.btnConnect.disabled = false;
  elements.btnDisconnect.disabled = true;
  elements.btnFullscreen.disabled = true;

  if (state.statsInterval) {
    clearInterval(state.statsInterval);
    state.statsInterval = null;
  }
  elements.stats.style.display = 'none';
}

function onConnected() {
  console.log('[WebRTC] 连接成功');
  startStats();
}

function onDisconnected() {
  console.log('[WebRTC] 连接断开');
  disconnect();
}

// ==================== 统计信息 ====================
async function startStats() {
  elements.stats.style.display = 'block';

  let lastBytes = 0;
  let lastTime = Date.now();

  state.statsInterval = setInterval(async () => {
    if (!state.pc) return;

    const stats = await state.pc.getStats();
    let currentBytes = 0;
    let currentLatency = 0;

    stats.forEach(report => {
      if (report.type === 'inbound-rtp' && report.mediaType === 'video') {
        currentBytes += report.bytesReceived || 0;

        // 计算延迟
        if (report.currentRoundTripTime) {
          currentLatency = report.currentRoundTripTime * 1000;
        }
      }
    });

    const now = Date.now();
    const elapsed = (now - lastTime) / 1000;
    const bitrate = ((currentBytes - lastBytes) * 8 / elapsed / 1000000).toFixed(2);

    lastBytes = currentBytes;
    lastTime = now;

    elements.latency.textContent = currentLatency.toFixed(0);
    elements.bitrate.textContent = bitrate;
    elements.fps.textContent = stats.get(' framerate')?.framerate || '-';
  }, 1000);
}

// ==================== 控制功能 ====================
function sendMouseEvent(type, x, y, button = 0) {
  if (!state.dataChannel || state.dataChannel.readyState !== 'open') return;

  state.dataChannel.send(JSON.stringify({
    type: 'mouse',
    action: type,
    x: Math.round(x),
    y: Math.round(y),
    button: button
  }));
}

function sendKeyEvent(type, key, code) {
  if (!state.dataChannel || state.dataChannel.readyState !== 'open') return;

  state.dataChannel.send(JSON.stringify({
    type: 'keyboard',
    action: type,
    key: key,
    code: code
  }));
}

// 绑定鼠标事件到视频
elements.remoteVideo.addEventListener('mousemove', (e) => {
  const rect = elements.remoteVideo.getBoundingClientRect();
  const x = (e.clientX - rect.left) * (elements.remoteVideo.videoWidth / rect.width);
  const y = (e.clientY - rect.top) * (elements.remoteVideo.videoHeight / rect.height);
  sendMouseEvent('move', x, y);
});

elements.remoteVideo.addEventListener('mousedown', (e) => {
  const rect = elements.remoteVideo.getBoundingClientRect();
  const x = (e.clientX - rect.left) * (elements.remoteVideo.videoWidth / rect.width);
  const y = (e.clientY - rect.top) * (elements.remoteVideo.videoHeight / rect.height);
  sendMouseEvent('down', x, y, e.button);
});

elements.remoteVideo.addEventListener('mouseup', (e) => {
  const rect = elements.remoteVideo.getBoundingClientRect();
  const x = (e.clientX - rect.left) * (elements.remoteVideo.videoWidth / rect.width);
  const y = (e.clientY - rect.top) * (elements.remoteVideo.videoHeight / rect.height);
  sendMouseEvent('up', x, y, e.button);
});

// 绑定键盘事件
document.addEventListener('keydown', (e) => {
  if (document.activeElement === elements.remoteVideo || document.activeElement === document.body) {
    e.preventDefault();
    sendKeyEvent('down', e.key, e.code);
  }
});

document.addEventListener('keyup', (e) => {
  if (document.activeElement === elements.remoteVideo || document.activeElement === document.body) {
    e.preventDefault();
    sendKeyEvent('up', e.key, e.code);
  }
});

// ==================== UI 事件 ====================
elements.btnConnect.addEventListener('click', connect);
elements.btnDisconnect.addEventListener('click', disconnect);
elements.btnFullscreen.addEventListener('click', () => {
  if (elements.remoteVideo.requestFullscreen) {
    elements.remoteVideo.requestFullscreen();
  }
});

// ==================== 工具函数 ====================
function updateStatus(status) {
  const statusMap = {
    'offline': { text: '离线', class: '' },
    'connecting': { text: '连接中...', class: 'connecting' },
    'online': { text: '已连接', class: 'online' }
  };

  const s = statusMap[status] || statusMap.offline;
  elements.statusText.textContent = s.text;
  elements.statusDot.className = 'status-dot ' + s.class;
}

// ==================== 初始化 ====================
connectWebSocket();
