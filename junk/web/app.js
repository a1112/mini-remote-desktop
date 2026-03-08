import { buildWsUrl } from './ws-url.js';
import { WebRtcMediaClient } from './transport/webrtc-media.js';
import { WebTransportMediaClient } from './transport/webtransport-media.js';
import { AdaptiveTuner } from './transport/adaptive-tuner.js';
import {
  buildOfferTransportPayload,
  buildWebCapabilities,
  canUseWebTransport,
  getPreferredTransportFromQuery,
  pickMediaTransport,
} from './transport/policy.js';

const CONFIG = {
  WS_URL: buildWsUrl(window.location),
  PREFERRED_TRANSPORT: getPreferredTransportFromQuery(window.location),
  CONNECT_TIMEOUT_MS: 1200,
  ICE_SERVERS: [
    { urls: 'stun:stun.l.google.com:19302' },
    { urls: 'stun:stun1.l.google.com:19302' },
  ]
};

const state = {
  ws: null,
  deviceId: null,
  devices: new Map(),
  selectedDevice: null,
  webrtc: null,
  webtransport: null,
  remoteStream: null,
  statsInterval: null,
  selectedTransport: null,
  mediaPath: null,
  wtStats: {
    bytes: 0,
    frameCount: 0,
    lastFrameAt: 0,
  },
  webrtcStats: {
    lastFramesDecoded: 0,
  },
  backendStats: {
    nackPerSec: 0,
    queueDepth: 0,
    targetBitrateKbps: 0,
    updatedAt: 0,
  },
  tuner: new AdaptiveTuner(),
};

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
  btnFullscreen: document.getElementById('btnFullscreen'),
};

const remoteCanvas = document.createElement('canvas');
remoteCanvas.id = 'remoteCanvas';
remoteCanvas.style.display = 'none';
remoteCanvas.style.maxWidth = '100%';
remoteCanvas.style.maxHeight = '100%';
remoteCanvas.style.background = '#000';
elements.remoteVideo.parentElement?.appendChild(remoteCanvas);

function log(level, msg, extra) {
  if (extra !== undefined) {
    console[level](`[media] ${msg}`, extra);
  } else {
    console[level](`[media] ${msg}`);
  }
}

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
        send({
          type: 'device',
          action: 'register',
          payload: {
            type: 'controller',
            name: 'Web 控制端',
            protocolVersion: 2,
            transports: ['webtransport', 'webrtc'],
            capabilities: buildWebCapabilities()
          }
        });
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

function sendCaptureUpdate(capturePatch) {
  if (!state.selectedDevice) return;
  send({
    type: 'control',
    action: 'updateCapture',
    payload: {
      targetDeviceId: state.selectedDevice,
      capture: capturePatch,
    }
  });
}

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

async function createWebRtcSession() {
  const webrtc = new WebRtcMediaClient({
    iceServers: CONFIG.ICE_SERVERS,
    onIceCandidate: (candidate) => {
      if (!candidate) return;
      send({
        type: 'webrtc',
        action: 'iceCandidate',
        payload: {
          targetDeviceId: state.selectedDevice,
          candidate,
        }
      });
    },
    onTrack: (stream) => {
      log('info', 'WebRTC media track attached');
      state.remoteStream = stream;
      elements.remoteVideo.srcObject = stream;
      elements.remoteVideo.style.display = 'block';
      remoteCanvas.style.display = 'none';
      elements.placeholder.style.display = 'none';
      state.mediaPath = 'webrtc';
    },
    onConnectionState: (connectionState) => {
      log('info', `WebRTC connection state: ${connectionState}`);
      if (connectionState === 'connected') onConnected();
      if (connectionState === 'disconnected' || connectionState === 'failed') onDisconnected();
    },
    onIceState: (iceState) => {
      log('info', `WebRTC ICE state: ${iceState}`);
    },
    onDataChannelOpen: () => log('info', 'DataChannel(control) opened'),
    onDataChannelMessage: (raw) => {
      try {
        const data = JSON.parse(raw);
        if (data?.type === 'agentStats' && data?.payload) {
          state.backendStats.nackPerSec = Number(data.payload.nackPerSec || 0);
          state.backendStats.queueDepth = Number(data.payload.queueDepth || 0);
          state.backendStats.targetBitrateKbps = Number(data.payload.targetBitrateKbps || 0);
          state.backendStats.updatedAt = Date.now();
        } else {
          log('info', `DataChannel message: ${data.type || 'unknown'}`);
        }
      } catch (_) {}
    }
  });
  webrtc.createControlChannel();
  return webrtc;
}

async function connect() {
  if (!state.selectedDevice) return;

  const device = state.devices.get(state.selectedDevice);
  if (!device) return;

  log('info', `connecting device: ${device.name}`);
  await disconnectMediaOnly();
  state.webrtc = await createWebRtcSession();

  const offer = await state.webrtc.createOffer();
  const transportPayload = buildOfferTransportPayload(CONFIG.PREFERRED_TRANSPORT);
  state.selectedTransport = transportPayload.preferredTransport;

  send({
    type: 'webrtc',
    action: 'offer',
    payload: {
      targetDeviceId: state.selectedDevice,
      offer,
      ...transportPayload,
      capabilities: buildWebCapabilities(),
    }
  });
  log('info', `offer sent with preferred transport=${state.selectedTransport}`);

  elements.btnConnect.disabled = true;
  elements.btnDisconnect.disabled = false;
  elements.btnFullscreen.disabled = false;
}

async function tryStartWebTransport(answerPayload) {
  const endpoint = answerPayload?.webtransport;
  if (!endpoint || !endpoint.url) {
    log('warn', 'answer has no webtransport endpoint; skip');
    return false;
  }
  if (!canUseWebTransport(window)) {
    log('warn', 'browser does not support WebTransport + WebCodecs');
    return false;
  }

  try {
    state.webtransport = new WebTransportMediaClient({
      endpoint,
      canvas: remoteCanvas,
      onStats: (s) => {
        state.wtStats.bytes = s.bytes || state.wtStats.bytes;
        state.wtStats.frameCount = s.frameCount || state.wtStats.frameCount;
        state.wtStats.lastFrameAt = s.lastFrameAt || state.wtStats.lastFrameAt;
      },
      onLog: (level, message) => log(level, message),
    });
    await state.webtransport.connect(CONFIG.CONNECT_TIMEOUT_MS);
    elements.remoteVideo.srcObject = null;
    elements.remoteVideo.style.display = 'none';
    remoteCanvas.style.display = 'block';
    elements.placeholder.style.display = 'none';
    state.mediaPath = 'webtransport';
    onConnected();
    log('info', 'media path switched to webtransport');
    return true;
  } catch (e) {
    log('warn', `webtransport connect failed: ${e?.message || e}`);
    await state.webtransport?.close();
    state.webtransport = null;
    return false;
  }
}

async function handleWebRTCMessage(action, payload) {
  log('info', `signal webrtc action=${action}`);
  switch (action) {
    case 'answer': {
      try {
        await state.webrtc?.setRemoteAnswer(payload.answer);
        log('info', 'remote answer applied');
      } catch (e) {
        console.error('[WebRTC] RemoteDescription 错误:', e);
        break;
      }
      const picked = pickMediaTransport(payload);
      state.selectedTransport = picked.selectedTransport;
      if (picked.selectedTransport === 'webtransport') {
        const ok = await tryStartWebTransport(payload);
        if (!ok) {
          state.mediaPath = 'webrtc';
          log('warn', 'fallback to webrtc media path');
        }
      } else {
        state.mediaPath = 'webrtc';
      }
      break;
    }

    case 'iceCandidate':
      try {
        await state.webrtc?.addIceCandidate(payload.candidate);
      } catch (e) {
        console.error('[WebRTC] ICE candidate 错误:', e, payload.candidate);
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

async function disconnectMediaOnly() {
  try {
    state.webrtc?.close();
  } catch (_) {}
  state.webrtc = null;
  try {
    await state.webtransport?.close();
  } catch (_) {}
  state.webtransport = null;
}

async function disconnect() {
  await disconnectMediaOnly();

  elements.remoteVideo.srcObject = null;
  elements.remoteVideo.style.display = 'none';
  remoteCanvas.style.display = 'none';
  elements.placeholder.style.display = 'block';

  elements.btnConnect.disabled = false;
  elements.btnDisconnect.disabled = true;
  elements.btnFullscreen.disabled = true;

  if (state.statsInterval) {
    clearInterval(state.statsInterval);
    state.statsInterval = null;
  }
  elements.stats.style.display = 'none';
  state.mediaPath = null;
}

function onConnected() {
  log('info', `connected, mediaPath=${state.mediaPath || 'unknown'}, selectedTransport=${state.selectedTransport || 'unknown'}`);
  startStats();
}

function onDisconnected() {
  log('warn', 'disconnected');
  disconnect();
}

async function startStats() {
  if (state.statsInterval) {
    clearInterval(state.statsInterval);
    state.statsInterval = null;
  }
  elements.stats.style.display = 'block';

  let lastBytes = 0;
  let lastTime = Date.now();
  let lastFrameCount = state.wtStats.frameCount;
  let lastFramesDecoded = state.webrtcStats.lastFramesDecoded || 0;

  state.statsInterval = setInterval(async () => {
    let currentBytes = 0;
    let currentLatency = 0;
    let currentLossPct = null;
    let fpsWindow = 0;
    if (state.mediaPath === 'webrtc' && state.webrtc?.pc) {
      try {
        const stats = await state.webrtc.pc.getStats();
        let packetsReceived = 0;
        let packetsLost = 0;
        let fpsFromReport = 0;
        let framesDecoded = 0;
        stats.forEach(report => {
          const isInboundVideo =
            report.type === 'inbound-rtp' &&
            (report.mediaType === 'video' || report.kind === 'video');
          if (isInboundVideo) {
            currentBytes += report.bytesReceived || 0;
            packetsReceived += report.packetsReceived || 0;
            packetsLost += report.packetsLost || 0;
            fpsFromReport = Math.max(fpsFromReport, report.framesPerSecond || 0);
            framesDecoded += report.framesDecoded || 0;
          }
          if (report.type === 'candidate-pair' && report.state === 'succeeded') {
            if (report.currentRoundTripTime) {
              currentLatency = Math.max(currentLatency, report.currentRoundTripTime * 1000);
            }
          }
        });
        const totalPackets = packetsReceived + packetsLost;
        if (totalPackets > 0) {
          currentLossPct = (packetsLost / totalPackets) * 100;
        }
        if (fpsFromReport > 0) {
          fpsWindow = fpsFromReport;
        } else {
          const nowTs = Date.now();
          const elapsed = Math.max((nowTs - lastTime) / 1000, 0.001);
          fpsWindow = Math.max(0, framesDecoded - lastFramesDecoded) / elapsed;
        }
        lastFramesDecoded = framesDecoded;
        state.webrtcStats.lastFramesDecoded = framesDecoded;
      } catch (e) {
        log('warn', `webrtc getStats failed: ${e?.message || e}`);
      }
    } else if (state.mediaPath === 'webtransport') {
      currentBytes = state.wtStats.bytes;
      const now = performance.now();
      if (state.wtStats.lastFrameAt > 0) {
        currentLatency = Math.max(0, now - state.wtStats.lastFrameAt);
      }
      currentLossPct = null;
    }

    const now = Date.now();
    const elapsed = (now - lastTime) / 1000;
    if (state.mediaPath === 'webtransport') {
      fpsWindow = Math.max(0, state.wtStats.frameCount - lastFrameCount) / Math.max(elapsed, 0.001);
    }
    const bitrate = ((currentBytes - lastBytes) * 8 / elapsed / 1000000).toFixed(2);

    lastBytes = currentBytes;
    lastTime = now;
    lastFrameCount = state.wtStats.frameCount;

    if (state.mediaPath === 'webtransport') {
      const stallMs = state.wtStats.lastFrameAt > 0 ? Math.max(0, performance.now() - state.wtStats.lastFrameAt) : 0;
      const patch = state.tuner.update({
        nowMs: now,
        fps: fpsWindow,
        stallMs,
        lossBurst: 0,
        backend: {
          nackBurst: state.backendStats.nackPerSec,
          queueDepth: state.backendStats.queueDepth,
        },
      });
      if (patch) {
        sendCaptureUpdate(patch);
        log('info', `adaptive tune bitrateKbps=${patch.bitrateKbps}`);
      }
    }

    elements.latency.textContent = currentLatency.toFixed(0);
    elements.bitrate.textContent = bitrate;
    elements.fps.textContent = fpsWindow > 0 ? fpsWindow.toFixed(1) : '-';
    elements.packetLoss.textContent = currentLossPct === null ? '-' : currentLossPct.toFixed(2);
  }, 1000);
}

function sendMouseEvent(type, x, y, button = 0) {
  state.webrtc?.sendControl({
    type: 'mouse',
    action: type,
    x: Math.round(x),
    y: Math.round(y),
    button: button
  });
}

function sendKeyEvent(type, key, code) {
  state.webrtc?.sendControl({
    type: 'keyboard',
    action: type,
    key: key,
    code: code
  });
}

function mediaRect() {
  if (state.mediaPath === 'webtransport') return remoteCanvas.getBoundingClientRect();
  return elements.remoteVideo.getBoundingClientRect();
}

function mediaResolution() {
  if (state.mediaPath === 'webtransport') {
    return { w: remoteCanvas.width || 1, h: remoteCanvas.height || 1 };
  }
  return { w: elements.remoteVideo.videoWidth || 1, h: elements.remoteVideo.videoHeight || 1 };
}

function bindPointerEvents(targetEl) {
  targetEl.addEventListener('mousemove', (e) => {
    const rect = mediaRect();
    const { w, h } = mediaResolution();
    const x = (e.clientX - rect.left) * (w / Math.max(1, rect.width));
    const y = (e.clientY - rect.top) * (h / Math.max(1, rect.height));
    sendMouseEvent('move', x, y);
  });

  targetEl.addEventListener('mousedown', (e) => {
    const rect = mediaRect();
    const { w, h } = mediaResolution();
    const x = (e.clientX - rect.left) * (w / Math.max(1, rect.width));
    const y = (e.clientY - rect.top) * (h / Math.max(1, rect.height));
    sendMouseEvent('down', x, y, e.button);
  });

  targetEl.addEventListener('mouseup', (e) => {
    const rect = mediaRect();
    const { w, h } = mediaResolution();
    const x = (e.clientX - rect.left) * (w / Math.max(1, rect.width));
    const y = (e.clientY - rect.top) * (h / Math.max(1, rect.height));
    sendMouseEvent('up', x, y, e.button);
  });
}

bindPointerEvents(elements.remoteVideo);
bindPointerEvents(remoteCanvas);

document.addEventListener('keydown', (e) => {
  if (document.activeElement === elements.remoteVideo || document.activeElement === document.body || document.activeElement === remoteCanvas) {
    e.preventDefault();
    sendKeyEvent('down', e.key, e.code);
  }
});

document.addEventListener('keyup', (e) => {
  if (document.activeElement === elements.remoteVideo || document.activeElement === document.body || document.activeElement === remoteCanvas) {
    e.preventDefault();
    sendKeyEvent('up', e.key, e.code);
  }
});

elements.btnConnect.addEventListener('click', connect);
elements.btnDisconnect.addEventListener('click', disconnect);
elements.btnFullscreen.addEventListener('click', () => {
  const active = state.mediaPath === 'webtransport' ? remoteCanvas : elements.remoteVideo;
  if (active.requestFullscreen) active.requestFullscreen();
});

function updateStatus(status) {
  const statusMap = {
    offline: { text: '离线', class: '' },
    connecting: { text: '连接中...', class: 'connecting' },
    online: { text: '已连接', class: 'online' }
  };
  const s = statusMap[status] || statusMap.offline;
  elements.statusText.textContent = s.text;
  elements.statusDot.className = 'status-dot ' + s.class;
}

connectWebSocket();
