/**
 * Mini Remote Desktop - 被控端 Agent
 *
 * 功能：
 * - 屏幕捕获与编码
 * - 鼠标键盘模拟
 * - WebRTC 连接
 */

const { app, BrowserWindow, desktopCapturer, ipcMain } = require('electron');
const path = require('path');
const robot = require('robotjs');
const WebSocket = require('ws');
const { loadRuntimeConfig } = require('./config');

let mainWindow;
let ws;
let deviceId;

// 配置
const CONFIG = loadRuntimeConfig();

// ==================== 创建窗口 ====================
function createWindow() {
  mainWindow = new BrowserWindow({
    width: 400,
    height: 300,
    webPreferences: {
      nodeIntegration: true,
      contextIsolation: false
    },
    icon: path.join(__dirname, 'icon.png'),
    autoHideMenuBar: true,
    resizable: false
  });

  mainWindow.loadFile('index.html');

  // 窗口准备好后连接服务器
  mainWindow.webContents.on('did-finish-load', () => {
    connectToServer();
  });

  // 开发模式下打开 DevTools
  // mainWindow.webContents.openDevTools();
}

// ==================== WebSocket 连接 ====================
function connectToServer() {
  ws = new WebSocket(CONFIG.wsUrl);
  updateStatus('connecting');

  ws.on('open', () => {
    console.log('[WS] 已连接服务器');
  });

  ws.on('message', async (data) => {
    try {
      const message = JSON.parse(data);
      handleMessage(message);
    } catch (err) {
      console.error('[WS] 消息错误:', err);
    }
  });

  ws.on('close', () => {
    console.log('[WS] 连接断开，3秒后重连...');
    updateStatus('disconnected');
    setTimeout(connectToServer, 3000);
  });

  ws.on('error', (err) => {
    console.error('[WS] 错误:', err);
  });
}

function handleMessage(message) {
  const { type, action, payload } = message;

  switch (type) {
    case 'system':
      if (action === 'connected') {
        deviceId = payload.deviceId;
        // 注册为被控端
        const hostname = require('os').hostname();
        send({
          type: 'device',
          action: 'register',
          payload: { type: 'agent', name: `${hostname} - Agent` }
        });
        updateStatus('registered');
      }
      break;

    case 'webrtc':
      handleWebRTCMessage(action, payload);
      break;
  }
}

function send(message) {
  if (ws && ws.readyState === 1) {
    ws.send(JSON.stringify(message));
  }
}

// ==================== WebRTC 处理 ====================
async function handleWebRTCMessage(action, payload) {
  switch (action) {
    case 'offer':
      await handleOffer(payload);
      break;

    case 'iceCandidate':
      if (payload?.candidate) {
        mainWindow?.webContents.send('webrtc-remote-candidate', {
          candidate: payload.candidate
        });
      }
      break;
  }
}

async function handleOffer(payload) {
  const { offer, controllerId, sessionId } = payload;

  // 获取屏幕源
  const sources = await desktopCapturer.getSources({
    types: ['screen', 'window']
  });

  if (sources.length === 0) {
    console.error('[Screen] 没有找到屏幕源');
    return;
  }

  // 在渲染进程中创建 WebRTC 连接
  mainWindow.webContents.send('start-webrtc', {
    offer,
    controllerId,
    sources: sources.map(s => ({ id: s.id, name: s.name, thumbnail: s.thumbnail.toDataURL() }))
  });
}

// ==================== IPC 通信 ====================
ipcMain.on('webrtc-answer', async (event, data) => {
  const { answer, controllerId } = data;

  // 发送 answer 给控制器
  send({
    type: 'webrtc',
    action: 'answer',
    payload: { answer, controllerId }
  });
});

ipcMain.on('control-message', (event, data) => {
  handleControlMessage(data);
});

ipcMain.on('webrtc-candidate', (event, data) => {
  const { candidate, controllerId } = data;
  send({
    type: 'webrtc',
    action: 'iceCandidate',
    payload: { targetDeviceId: controllerId, candidate }
  });
});

function handleControlMessage(data) {
  const { type, action } = data;

  switch (type) {
    case 'mouse':
      handleMouse(action, data);
      break;

    case 'keyboard':
      handleKeyboard(action, data);
      break;
  }
}

function handleMouse(action, data) {
  const { x, y, button } = data;

  switch (action) {
    case 'move':
      robot.moveMouse(Math.round(x), Math.round(y));
      break;

    case 'down':
      robot.mouseClick(button === 2 ? 'right' : 'left');
      break;

    case 'up':
      // robot.mouseToggle('up', button === 2 ? 'right' : 'left');
      break;

    case 'scroll':
      robot.scrollMouse(0, data.delta);
      break;
  }
}

function handleKeyboard(action, data) {
  const { key, code } = data;

  switch (action) {
    case 'down':
      robot.keyToggle(key, 'down');
      break;

    case 'up':
      robot.keyToggle(key, 'up');
      break;
  }
}

// ==================== 状态更新 ====================
function updateStatus(status) {
  mainWindow?.webContents.send('status', status);
}

// ==================== 应用生命周期 ====================
app.whenReady().then(createWindow);

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow();
  }
});

app.on('before-quit', () => {
  if (ws) ws.close();
});
