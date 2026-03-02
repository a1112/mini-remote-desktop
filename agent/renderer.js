/**
 * Mini Remote Agent - 渲染进程
 *
 * 处理 UI 和 WebRTC 连接
 */

const { ipcRenderer } = require('electron');
const { loadRuntimeConfig } = require('./config');

let peerConnection;
let localStream;
let selectedSourceId = null;
let currentSessionId = null;
let currentControllerId = null;
const pendingRemoteCandidates = [];
const CONFIG = loadRuntimeConfig();

// ==================== 状态更新 ====================
ipcRenderer.on('status', (event, status) => {
  const statusMap = {
    'connecting': { ws: '连接服务器中...', conn: '等待连接', wsClass: 'connecting', connClass: '' },
    'registered': { ws: '已注册', conn: '等待连接', wsClass: 'registered', connClass: '' },
    'connected': { ws: '在线', conn: '已连接', wsClass: 'registered', connClass: 'connected' },
    'disconnected': { ws: '断开连接', conn: '等待连接', wsClass: 'disconnected', connClass: '' }
  };

  const s = statusMap[status] || statusMap.connecting;
  document.getElementById('wsStatus').textContent = s.ws;
  document.getElementById('connStatus').textContent = s.conn;
  document.getElementById('wsDot').className = 'dot ' + s.wsClass;
  document.getElementById('connDot').className = 'dot ' + s.connClass;
});

// ==================== WebRTC 连接 ====================
ipcRenderer.on('start-webrtc', async (event, data) => {
  const { offer, controllerId, sources } = data;

  // 显示屏幕选择界面
  showScreenSelection(sources, offer, controllerId);
});

ipcRenderer.on('webrtc-remote-candidate', async (event, data) => {
  const candidate = data?.candidate;
  if (!candidate) return;

  if (!peerConnection || !peerConnection.remoteDescription) {
    pendingRemoteCandidates.push(candidate);
    return;
  }

  try {
    await peerConnection.addIceCandidate(new RTCIceCandidate(candidate));
  } catch (err) {
    console.error('[Agent] 添加远端 ICE 失败:', err);
  }
});

function showScreenSelection(sources, offer, controllerId) {
  const screenList = document.getElementById('screenList');
  const selectScreen = document.getElementById('selectScreen');

  // 保存 offer 和 controllerId 供后续使用
  currentControllerId = controllerId;

  screenList.innerHTML = sources.map(s => `
    <div class="screen-item" data-id="${s.id}">
      <img src="${s.thumbnail}" class="screen-thumb">
      <div class="screen-name">${s.name}</div>
    </div>
  `).join('');

  selectScreen.style.display = 'block';

  // 绑定点击事件
  screenList.querySelectorAll('.screen-item').forEach(el => {
    el.addEventListener('click', () => {
      const sourceId = el.dataset.id;
      startWebRTC(sourceId, offer, controllerId);
    });
  });
}

async function startWebRTC(sourceId, offer, controllerId) {
  selectedSourceId = sourceId;
  currentControllerId = controllerId;

  // 隐藏选择界面
  document.getElementById('selectScreen').style.display = 'none';

  // 获取屏幕流
  try {
    localStream = await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: {
        mandatory: {
          chromeMediaSource: 'desktop',
          chromeMediaSourceId: sourceId,
          minWidth: CONFIG.capture.minWidth,
          maxWidth: CONFIG.capture.maxWidth,
          minHeight: CONFIG.capture.minHeight,
          maxHeight: CONFIG.capture.maxHeight,
          frameRate: CONFIG.capture.fps
        }
      }
    });

    console.log('[Agent] 获取屏幕流成功');

    // 创建 RTCPeerConnection
    peerConnection = new RTCPeerConnection({
      iceServers: [
        { urls: 'stun:stun.l.google.com:19302' }
      ]
    });

    // 添加视频轨道
    localStream.getTracks().forEach(track => {
      console.log('[Agent] 添加轨道:', track.kind);
      peerConnection.addTrack(track, localStream);
    });

    // 监听 ICE 候选
    peerConnection.onicecandidate = (event) => {
      if (event.candidate) {
        ipcRenderer.send('webrtc-candidate', {
          candidate: event.candidate,
          controllerId
        });
      }
    };

    peerConnection.ondatachannel = (event) => {
      setupDataChannel(event.channel);
    };

    // 设置远程描述
    await peerConnection.setRemoteDescription(new RTCSessionDescription(offer));
    console.log('[Agent] RemoteDescription 设置成功');
    await flushPendingCandidates();

    // 创建应答
    const answer = await peerConnection.createAnswer();
    await peerConnection.setLocalDescription(answer);
    console.log('[Agent] Answer 创建成功');

    // 发送应答给主进程
    ipcRenderer.send('webrtc-answer', { answer, controllerId });

    // 更新连接状态
    document.getElementById('connStatus').textContent = '屏幕共享中';
    document.getElementById('connDot').className = 'dot connected';

  } catch (err) {
    console.error('启动 WebRTC 失败:', err);
    document.getElementById('connStatus').textContent = '启动失败: ' + err.message;
  }
}

function setupDataChannel(channel) {
  channel.onopen = () => {
    console.log('[Agent] DataChannel 已打开');
  };

  channel.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data);
      ipcRenderer.send('control-message', data);
    } catch (err) {
      console.error('[Agent] DataChannel 消息解析失败:', err);
    }
  };
}

async function flushPendingCandidates() {
  if (!peerConnection || !peerConnection.remoteDescription) return;

  while (pendingRemoteCandidates.length > 0) {
    const candidate = pendingRemoteCandidates.shift();
    try {
      await peerConnection.addIceCandidate(new RTCIceCandidate(candidate));
    } catch (err) {
      console.error('[Agent] 补充 ICE 失败:', err);
    }
  }
}
